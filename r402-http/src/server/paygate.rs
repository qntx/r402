//! Core payment gate logic for enforcing x402 payments (V2-only).
//!
//! The [`Paygate`] struct handles the full payment lifecycle:
//! extracting headers, verifying with the facilitator, settling on-chain,
//! and returning 402 responses when payment is required.

use axum_core::body::Body;
use axum_core::extract::Request;
use axum_core::response::{IntoResponse, Response};
use http::{HeaderMap, HeaderValue, StatusCode};
use r402::facilitator::Facilitator;
use r402::proto;
use r402::proto::Base64Bytes;
use r402::proto::v2;
use serde_json::json;
use std::convert::Infallible;
use std::sync::Arc;
use tower::Service;
use url::Url;

#[cfg(feature = "telemetry")]
use tracing::Instrument;
#[cfg(feature = "telemetry")]
use tracing::instrument;

use super::error::{PaygateError, VerificationError};

/// Builder for resource information that can be used with both V1 and V2 protocols.
#[derive(Debug, Clone)]
pub struct ResourceInfoBuilder {
    /// Description of the protected resource
    pub description: String,
    /// MIME type of the protected resource
    pub mime_type: String,
    /// Optional explicit URL of the protected resource
    pub url: Option<String>,
}

impl Default for ResourceInfoBuilder {
    fn default() -> Self {
        Self {
            description: String::new(),
            mime_type: "application/json".to_string(),
            url: None,
        }
    }
}

impl ResourceInfoBuilder {
    /// Determines the resource URL (static or dynamic).
    ///
    /// If `url` is set, returns it directly. Otherwise, constructs a URL by combining
    /// the base URL with the request URI's path and query.
    ///
    /// # Panics
    ///
    /// Panics if internal URL construction fails (should not happen in practice).
    #[allow(clippy::unwrap_used)]
    pub fn as_resource_info(&self, base_url: Option<&Url>, req: &Request) -> v2::ResourceInfo {
        let url = self.url.clone().unwrap_or_else(|| {
            let mut url = base_url.cloned().unwrap_or_else(|| {
                let host = req.headers().get("host").and_then(|h| h.to_str().ok()).unwrap_or("localhost");
                let origin = format!("http://{host}");
                let url = Url::parse(&origin).unwrap_or_else(|_| Url::parse("http://localhost").unwrap());
                #[cfg(feature = "telemetry")]
                tracing::warn!(
                    "X402Middleware base_url is not configured; using {url} as origin for resource resolution"
                );
                url
            });
            let request_uri = req.uri();
            url.set_path(request_uri.path());
            url.set_query(request_uri.query());
            url.to_string()
        });
        v2::ResourceInfo {
            description: self.description.clone(),
            mime_type: self.mime_type.clone(),
            url,
        }
    }
}

/// V2-only payment gate for enforcing x402 payments.
///
/// Handles the full payment lifecycle: header extraction, verification,
/// settlement, and 402 response generation using the V2 wire format.
///
/// To add lifecycle hooks (before/after verify and settle), wrap your
/// facilitator with [`HookedFacilitator`](r402::hooks::HookedFacilitator)
/// before passing it to the payment gate.
#[allow(missing_debug_implementations)]
pub struct Paygate<TFacilitator> {
    /// The facilitator for verifying and settling payments
    pub facilitator: TFacilitator,
    /// Whether to settle before or after request execution
    pub settle_before_execution: bool,
    /// Accepted V2 payment requirements
    pub accepts: Arc<Vec<v2::PriceTag>>,
    /// Resource information for the protected endpoint
    pub resource: v2::ResourceInfo,
}

/// The V2 payment header name.
const PAYMENT_HEADER_NAME: &str = "Payment-Signature";

/// The V2 payment payload type.
type V2PaymentPayload = v2::PaymentPayload<v2::PaymentRequirements, serde_json::Value>;

impl<TFacilitator> Paygate<TFacilitator> {
    /// Calls the inner service with proper telemetry instrumentation.
    async fn call_inner<
        ReqBody,
        ResBody,
        S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>>,
    >(
        mut inner: S,
        req: http::Request<ReqBody>,
    ) -> Result<http::Response<ResBody>, S::Error>
    where
        S::Future: Send,
    {
        #[cfg(feature = "telemetry")]
        {
            inner
                .call(req)
                .instrument(tracing::info_span!("inner"))
                .await
        }
        #[cfg(not(feature = "telemetry"))]
        {
            inner.call(req).await
        }
    }
}

impl<TFacilitator> Paygate<TFacilitator>
where
    TFacilitator: Facilitator + Sync,
{
    /// Handles an incoming request, processing payment if required.
    ///
    /// Returns 402 response if payment fails.
    /// Otherwise, returns the response from the inner service.
    ///
    /// # Errors
    ///
    /// This method is infallible (`Infallible` error type).
    #[cfg_attr(
        feature = "telemetry",
        instrument(name = "x402.handle_request", skip_all)
    )]
    pub async fn handle_request<
        ReqBody,
        ResBody,
        S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>>,
    >(
        self,
        inner: S,
        req: http::Request<ReqBody>,
    ) -> Result<Response, Infallible>
    where
        S::Response: IntoResponse,
        S::Error: IntoResponse,
        S::Future: Send,
    {
        match self.handle_request_fallible(inner, req).await {
            Ok(response) => Ok(response),
            Err(err) => Ok(error_into_response(err, &self.accepts, &self.resource)),
        }
    }

    /// Enriches price tags with facilitator capabilities (e.g., fee payer address).
    pub async fn enrich_accepts(&mut self) {
        let capabilities = self.facilitator.supported().await.unwrap_or_default();

        let accepts = (*self.accepts)
            .clone()
            .into_iter()
            .map(|mut pt| {
                pt.enrich(&capabilities);
                pt
            })
            .collect::<Vec<_>>();
        self.accepts = Arc::new(accepts);
    }

    /// Handles an incoming request, returning errors as `PaygateError`.
    ///
    /// This is the fallible version of `handle_request` that returns an actual error
    /// instead of turning it into 402 Payment Required response.
    ///
    /// # Errors
    ///
    /// Returns [`PaygateError`] if payment processing fails.
    pub async fn handle_request_fallible<
        ReqBody,
        ResBody,
        S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>>,
    >(
        &self,
        inner: S,
        req: http::Request<ReqBody>,
    ) -> Result<Response, PaygateError>
    where
        S::Response: IntoResponse,
        S::Error: IntoResponse,
        S::Future: Send,
    {
        let header = extract_payment_header(req.headers(), PAYMENT_HEADER_NAME).ok_or(
            VerificationError::PaymentHeaderRequired(PAYMENT_HEADER_NAME),
        )?;
        let payment_payload = extract_payment_payload::<V2PaymentPayload>(header)
            .ok_or(VerificationError::InvalidPaymentHeader)?;

        let verify_request = make_verify_request(payment_payload, &self.accepts)?;

        if self.settle_before_execution {
            #[cfg(feature = "telemetry")]
            tracing::debug!("Settling payment before request execution");

            let settlement = self
                .facilitator
                .settle(verify_request.into())
                .await
                .map_err(|e| PaygateError::Settlement(format!("{e}")))?;

            if let proto::SettleResponse::Error {
                reason, message, ..
            } = &settlement
            {
                let detail = message.as_deref().unwrap_or(reason.as_str());
                return Err(PaygateError::Settlement(detail.to_owned()));
            }

            let header_value = settlement_to_header(settlement)?;

            let response = match Self::call_inner(inner, req).await {
                Ok(response) => response,
                Err(err) => return Ok(err.into_response()),
            };

            let mut res = response;
            res.headers_mut().insert("Payment-Response", header_value);
            Ok(res.into_response())
        } else {
            #[cfg(feature = "telemetry")]
            tracing::debug!("Settling payment after request execution");

            let verify_response = self
                .facilitator
                .verify(verify_request.clone())
                .await
                .map_err(|e| VerificationError::VerificationFailed(format!("{e}")))?;

            validate_verify_response(verify_response)?;

            let response = match Self::call_inner(inner, req).await {
                Ok(response) => response,
                Err(err) => return Ok(err.into_response()),
            };

            if response.status().is_client_error() || response.status().is_server_error() {
                return Ok(response.into_response());
            }

            let settlement = self
                .facilitator
                .settle(verify_request.into())
                .await
                .map_err(|e| PaygateError::Settlement(format!("{e}")))?;

            if let proto::SettleResponse::Error {
                reason, message, ..
            } = &settlement
            {
                let detail = message.as_deref().unwrap_or(reason.as_str());
                return Err(PaygateError::Settlement(detail.to_owned()));
            }

            let header_value = settlement_to_header(settlement)?;

            let mut res = response;
            res.headers_mut().insert("Payment-Response", header_value);
            Ok(res.into_response())
        }
    }
}

/// Extracts the payment header value from the header map.
fn extract_payment_header<'a>(header_map: &'a HeaderMap, header_name: &'a str) -> Option<&'a [u8]> {
    header_map.get(header_name).map(HeaderValue::as_bytes)
}

/// Extracts and deserializes the payment payload from base64-encoded header bytes.
fn extract_payment_payload<T>(header_bytes: &[u8]) -> Option<T>
where
    T: serde::de::DeserializeOwned,
{
    let base64 = Base64Bytes::from(header_bytes).decode().ok()?;
    let value = serde_json::from_slice(base64.as_ref()).ok()?;
    Some(value)
}

/// Converts a [`proto::SettleResponse`] into an HTTP header value.
///
/// Returns an error response if conversion fails.
#[allow(clippy::needless_pass_by_value)] // settlement is consumed by serialization
fn settlement_to_header(settlement: proto::SettleResponse) -> Result<HeaderValue, PaygateError> {
    let json =
        serde_json::to_vec(&settlement).map_err(|err| PaygateError::Settlement(err.to_string()))?;
    let payment_header = Base64Bytes::encode(json);
    HeaderValue::from_bytes(payment_header.as_ref())
        .map_err(|err| PaygateError::Settlement(err.to_string()))
}

/// Constructs a V2 verify request from the payment payload and accepted requirements.
fn make_verify_request(
    payment_payload: V2PaymentPayload,
    accepts: &[v2::PriceTag],
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

/// Validates a verify response, rejecting invalid or unknown variants.
fn validate_verify_response(
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

/// Converts a [`PaygateError`] into a V2 402 Payment Required HTTP response.
fn error_into_response(
    err: PaygateError,
    accepts: &[v2::PriceTag],
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
        PaygateError::Settlement(ref err) => {
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
    }
}
