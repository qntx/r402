//! Version-specific protocol behavior for the x402 payment gate.
//!
//! The [`PaygateProtocol`] trait abstracts over V1 and V2 wire formats,
//! allowing the core [`super::paygate::Paygate`] logic to be shared.

use axum_core::body::Body;
use axum_core::response::Response;
use http::{HeaderValue, StatusCode};
use r402::proto;
use r402::proto::Base64Bytes;
use r402::proto::{SupportedResponse, v1, v2};
use serde_json::json;

use super::error::{PaygateError, VerificationError};

/// Trait defining version-specific behavior for the x402 payment gate.
///
/// This trait is implemented directly on the price tag types (`V1PriceTag` and
/// `V2PriceTag`/`v2::PaymentRequirements`), allowing the core payment gate logic
/// to be shared while version-specific behavior is implemented separately.
pub trait PaygateProtocol: Clone + Send + Sync + 'static {
    /// The payment payload type extracted from the request header.
    type PaymentPayload: serde::de::DeserializeOwned + Send;

    /// The HTTP header name for the payment payload.
    const PAYMENT_HEADER_NAME: &'static str;

    /// Constructs a verify request from the payment payload and accepted requirements.
    ///
    /// The `resource` parameter provides resource information that may be needed
    /// for protocol-specific requirements (e.g., V1 includes resource info in `PaymentRequirements`).
    /// # Errors
    ///
    /// Returns [`VerificationError`] if the request cannot be constructed.
    #[allow(clippy::needless_pass_by_value)]
    fn make_verify_request(
        payload: Self::PaymentPayload,
        accepts: &[Self],
        resource: &v2::ResourceInfo,
    ) -> Result<proto::VerifyRequest, VerificationError>;

    /// Converts an error into an HTTP response with appropriate format.
    fn error_into_response(
        err: PaygateError,
        accepts: &[Self],
        resource: &v2::ResourceInfo,
    ) -> Response;

    /// Converts the verify response to the protocol-specific format and validates it.
    /// # Errors
    ///
    /// Returns [`VerificationError`] if the response is invalid.
    #[allow(clippy::needless_pass_by_value)]
    fn validate_verify_response(
        verify_response: proto::VerifyResponse,
    ) -> Result<(), VerificationError>;

    /// Enriches a price tag with facilitator capabilities.
    ///
    /// Called by middleware when building 402 response to add extra information like fee payer
    /// from the facilitator's supported endpoints.
    fn enrich_with_capabilities(&mut self, capabilities: &SupportedResponse);
}

impl PaygateProtocol for v1::PriceTag {
    type PaymentPayload = v1::PaymentPayload;

    const PAYMENT_HEADER_NAME: &'static str = "X-PAYMENT";

    fn make_verify_request(
        payment_payload: Self::PaymentPayload,
        accepts: &[Self],
        resource: &v2::ResourceInfo,
    ) -> Result<proto::VerifyRequest, VerificationError> {
        let selected = accepts
            .iter()
            .find(|requirement| {
                requirement.scheme == payment_payload.scheme
                    && requirement.network == payment_payload.network
            })
            .ok_or(VerificationError::NoPaymentMatching)?;

        let verify_request = v1::VerifyRequest {
            x402_version: v1::V1,
            payment_payload,
            payment_requirements: price_tag_to_v1_requirements(selected, resource),
        };

        verify_request
            .try_into()
            .map_err(|e| VerificationError::VerificationFailed(format!("{e}")))
    }

    fn error_into_response(
        err: PaygateError,
        accepts: &[Self],
        resource: &v2::ResourceInfo,
    ) -> Response {
        match err {
            PaygateError::Verification(err) => {
                let payment_required_response = v1::PaymentRequired {
                    error: Some(err.to_string()),
                    accepts: accepts
                        .iter()
                        .map(|pt| price_tag_to_v1_requirements(pt, resource))
                        .collect(),
                    x402_version: v1::V1,
                };
                let payment_required_response_bytes =
                    serde_json::to_vec(&payment_required_response).expect("serialization failed");
                let body = Body::from(payment_required_response_bytes);
                Response::builder()
                    .status(StatusCode::PAYMENT_REQUIRED)
                    .header("Content-Type", "application/json")
                    .body(body)
                    .expect("Fail to construct response")
            }
            PaygateError::Settlement(err) => settlement_error_response(err),
        }
    }

    fn validate_verify_response(
        verify_response: proto::VerifyResponse,
    ) -> Result<(), VerificationError> {
        validate_verify_response_common(verify_response)
    }

    fn enrich_with_capabilities(&mut self, capabilities: &SupportedResponse) {
        self.enrich(capabilities);
    }
}

impl PaygateProtocol for v2::PriceTag {
    type PaymentPayload = v2::PaymentPayload<v2::PaymentRequirements, serde_json::Value>;

    const PAYMENT_HEADER_NAME: &'static str = "Payment-Signature";

    fn make_verify_request(
        payment_payload: Self::PaymentPayload,
        accepts: &[Self],
        _resource: &v2::ResourceInfo,
    ) -> Result<proto::VerifyRequest, VerificationError> {
        let accepted = &payment_payload.accepted;

        let selected = accepts
            .iter()
            .find(|price_tag| **price_tag == *accepted)
            .ok_or(VerificationError::NoPaymentMatching)?;

        let verify_request = v2::VerifyRequest {
            x402_version: v2::V2,
            payment_payload,
            payment_requirements: selected.requirements.clone(),
        };

        let json = serde_json::to_value(&verify_request)
            .map_err(|e| VerificationError::VerificationFailed(format!("{e}")))?;

        Ok(proto::VerifyRequest::from(json))
    }

    fn error_into_response(
        err: PaygateError,
        accepts: &[Self],
        resource: &v2::ResourceInfo,
    ) -> Response {
        match err {
            PaygateError::Verification(err) => {
                let payment_required_response = v2::PaymentRequired {
                    error: Some(err.to_string()),
                    accepts: accepts.iter().map(|pt| pt.requirements.clone()).collect(),
                    x402_version: v2::V2,
                    resource: resource.clone(),
                    extensions: None,
                };
                let payment_required_bytes =
                    serde_json::to_vec(&payment_required_response).expect("serialization failed");
                let payment_required_header = Base64Bytes::encode(&payment_required_bytes);
                let header_value = HeaderValue::from_bytes(payment_required_header.as_ref())
                    .expect("Failed to create header value");

                Response::builder()
                    .status(StatusCode::PAYMENT_REQUIRED)
                    .header("Payment-Required", header_value)
                    .body(Body::empty())
                    .expect("Fail to construct response")
            }
            PaygateError::Settlement(err) => settlement_error_response(err),
        }
    }

    fn validate_verify_response(
        verify_response: proto::VerifyResponse,
    ) -> Result<(), VerificationError> {
        validate_verify_response_common(verify_response)
    }

    fn enrich_with_capabilities(&mut self, capabilities: &SupportedResponse) {
        self.enrich(capabilities);
    }
}

/// Converts a `v1::PriceTag` to `v1::PaymentRequirements` with resource info.
fn price_tag_to_v1_requirements(
    price_tag: &v1::PriceTag,
    resource: &v2::ResourceInfo,
) -> v1::PaymentRequirements {
    v1::PaymentRequirements {
        scheme: price_tag.scheme.clone(),
        network: price_tag.network.clone(),
        max_amount_required: price_tag.amount.clone(),
        resource: resource.url.clone(),
        description: resource.description.clone(),
        mime_type: resource.mime_type.clone(),
        output_schema: None,
        pay_to: price_tag.pay_to.clone(),
        max_timeout_seconds: price_tag.max_timeout_seconds,
        asset: price_tag.asset.clone(),
        extra: price_tag.extra.clone(),
    }
}

/// Shared verification response validation for both V1 and V2.
fn validate_verify_response_common(
    verify_response: proto::VerifyResponse,
) -> Result<(), VerificationError> {
    match verify_response {
        proto::VerifyResponse::Valid { .. } => Ok(()),
        proto::VerifyResponse::Invalid { reason, .. } => {
            Err(VerificationError::VerificationFailed(reason))
        }
        _ => Err(VerificationError::VerificationFailed(
            "unknown verify response variant".into(),
        )),
    }
}

/// Shared settlement error response for both V1 and V2.
fn settlement_error_response(err: String) -> Response {
    let body = Body::from(
        json!({
            "error": "Settlement failed",
            "details": err
        })
        .to_string(),
    );
    Response::builder()
        .status(StatusCode::PAYMENT_REQUIRED)
        .header("Content-Type", "application/json")
        .body(body)
        .expect("Fail to construct response")
}
