//! Core payment gate logic for enforcing x402 payments (V2-only).
//!
//! The [`Paygate`] struct handles the full payment lifecycle:
//! extracting headers, verifying with the facilitator, settling on-chain,
//! and returning 402 responses when payment is required.

use std::convert::Infallible;
use std::sync::Arc;

use axum_core::body::Body;
use axum_core::extract::Request;
use axum_core::response::{IntoResponse, Response};
use http::{HeaderMap, HeaderValue, StatusCode};
use r402::facilitator::Facilitator;
use r402::proto;
use r402::proto::Base64Bytes;
use r402::proto::v2;
use serde_json::json;
use tower::Service;
#[cfg(feature = "telemetry")]
use tracing::{Instrument, instrument};
use url::Url;

use super::{PaygateError, VerificationError};

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
/// Construct via [`PaygateBuilder`] (obtained from [`Paygate::builder`]).
///
/// To add lifecycle hooks (before/after verify and settle), wrap your
/// facilitator with [`HookedFacilitator`](r402::hooks::HookedFacilitator)
/// before passing it to the payment gate.
#[allow(missing_debug_implementations)]
pub struct Paygate<TFacilitator> {
    pub(crate) facilitator: TFacilitator,
    pub(crate) accepts: Arc<Vec<v2::PriceTag>>,
    pub(crate) resource: v2::ResourceInfo,
}

/// Builder for constructing a [`Paygate`] with validated configuration.
///
/// # Example
///
/// ```ignore
/// let gate = Paygate::builder(facilitator)
///     .accept(price_tag)
///     .resource(resource_info)
///     .build();
/// ```
#[allow(missing_debug_implementations)]
pub struct PaygateBuilder<TFacilitator> {
    facilitator: TFacilitator,
    accepts: Vec<v2::PriceTag>,
    resource: Option<v2::ResourceInfo>,
}

impl<TFacilitator> Paygate<TFacilitator> {
    /// Returns a new builder seeded with the given facilitator.
    pub const fn builder(facilitator: TFacilitator) -> PaygateBuilder<TFacilitator> {
        PaygateBuilder {
            facilitator,
            accepts: Vec::new(),
            resource: None,
        }
    }

    /// Returns a reference to the underlying facilitator.
    pub const fn facilitator(&self) -> &TFacilitator {
        &self.facilitator
    }

    /// Returns a reference to the accepted price tags.
    pub fn accepts(&self) -> &[v2::PriceTag] {
        &self.accepts
    }

    /// Returns a reference to the resource information.
    pub const fn resource(&self) -> &v2::ResourceInfo {
        &self.resource
    }

    /// Converts a [`PaygateError`] into a proper 402 HTTP response using
    /// this gate's accepted price tags and resource information.
    ///
    /// This is the public convenience wrapper around [`error_into_response`],
    /// useful in composable flows where the caller handles verification
    /// separately from settlement.
    #[must_use]
    pub fn error_response(&self, err: PaygateError) -> Response {
        error_into_response(err, &self.accepts, &self.resource)
    }
}

impl<TFacilitator> PaygateBuilder<TFacilitator> {
    /// Adds a single accepted payment option.
    #[must_use]
    pub fn accept(mut self, price_tag: v2::PriceTag) -> Self {
        self.accepts.push(price_tag);
        self
    }

    /// Adds multiple accepted payment options.
    #[must_use]
    pub fn accepts(mut self, price_tags: impl IntoIterator<Item = v2::PriceTag>) -> Self {
        self.accepts.extend(price_tags);
        self
    }

    /// Sets the resource metadata returned in 402 responses.
    #[must_use]
    pub fn resource(mut self, resource: v2::ResourceInfo) -> Self {
        self.resource = Some(resource);
        self
    }

    /// Consumes the builder and produces a configured [`Paygate`].
    ///
    /// Uses empty resource info if none was provided.
    pub fn build(self) -> Paygate<TFacilitator> {
        Paygate {
            facilitator: self.facilitator,
            accepts: Arc::new(self.accepts),
            resource: self.resource.unwrap_or_else(|| v2::ResourceInfo {
                description: String::new(),
                mime_type: "application/json".to_owned(),
                url: String::new(),
            }),
        }
    }
}

/// The V2 payment header name.
pub const PAYMENT_HEADER_NAME: &str = "Payment-Signature";

/// The V2 payment payload type.
pub type V2PaymentPayload = v2::PaymentPayload<v2::PaymentRequirements, serde_json::Value>;

/// A verified payment token ready for on-chain settlement.
///
/// Produced by [`Paygate::verify_only`] after the facilitator confirms the
/// payment signature is valid. The caller controls *when* settlement happens:
///
/// - **Synchronous**: call [`settle`](Self::settle) immediately, then attach
///   the header via [`settlement_to_header`].
/// - **Deferred**: stream the response first, then settle and append the
///   result (e.g., as an SSE trailer event).
///
/// [`settle`](Self::settle) **consumes** `self`, preventing double-settlement
/// at the type level.
#[derive(Debug)]
pub struct VerifiedPayment {
    /// The settle request derived from the verified payment payload.
    settle_request: proto::SettleRequest,
}

impl VerifiedPayment {
    /// Execute on-chain settlement for this verified payment.
    ///
    /// Takes ownership of `self` to prevent double-settlement.
    ///
    /// # Errors
    ///
    /// Returns [`PaygateError::Settlement`] if the facilitator rejects the
    /// settlement or if the on-chain transaction fails.
    pub async fn settle<F: Facilitator>(
        self,
        facilitator: &F,
    ) -> Result<proto::SettleResponse, PaygateError> {
        let settlement = facilitator
            .settle(self.settle_request)
            .await
            .map_err(|e| PaygateError::Settlement(format!("{e}")))?;

        if let proto::SettleResponse::Error {
            reason, message, ..
        } = &settlement
        {
            let detail = message.as_deref().unwrap_or(reason.as_str());
            return Err(PaygateError::Settlement(detail.to_owned()));
        }

        Ok(settlement)
    }

    /// Returns a reference to the underlying settle request.
    #[must_use]
    pub const fn settle_request(&self) -> &proto::SettleRequest {
        &self.settle_request
    }
}

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
        &self,
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

    /// Verify the payment from request headers without executing the inner
    /// service or settling on-chain.
    ///
    /// Returns a [`VerifiedPayment`] token on success, which the caller can
    /// later [`settle`](VerifiedPayment::settle) at their discretion.
    ///
    /// This is the building block for **composable** payment flows such as
    /// deferred settlement for streaming responses.
    ///
    /// # Errors
    ///
    /// Returns [`PaygateError::Verification`] if the payment header is missing,
    /// malformed, or rejected by the facilitator.
    #[cfg_attr(feature = "telemetry", instrument(name = "x402.verify_only", skip_all))]
    pub async fn verify_only(&self, headers: &HeaderMap) -> Result<VerifiedPayment, PaygateError> {
        let header = extract_payment_header(headers, PAYMENT_HEADER_NAME).ok_or(
            VerificationError::PaymentHeaderRequired(PAYMENT_HEADER_NAME),
        )?;
        let payment_payload = extract_payment_payload::<V2PaymentPayload>(header)
            .ok_or(VerificationError::InvalidPaymentHeader)?;

        let verify_request = make_verify_request(payment_payload, &self.accepts)?;

        let verify_response = self
            .facilitator
            .verify(verify_request.clone())
            .await
            .map_err(|e| VerificationError::VerificationFailed(format!("{e}")))?;

        validate_verify_response(verify_response)?;

        Ok(VerifiedPayment {
            settle_request: verify_request.into(),
        })
    }

    /// Handles an incoming request, returning errors as `PaygateError`.
    ///
    /// This is the fallible version of [`handle_request`](Self::handle_request)
    /// that returns an actual error instead of converting it into a 402
    /// Payment Required response.
    ///
    /// Internally delegates to [`verify_only`](Self::verify_only) for
    /// verification and [`VerifiedPayment::settle`] for settlement.
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
        // Step 1: Verify the payment (borrows headers, does not consume req).
        let verified = self.verify_only(req.headers()).await?;

        // Step 2: Execute the inner handler.
        let response = match Self::call_inner(inner, req).await {
            Ok(response) => response,
            Err(err) => return Ok(err.into_response()),
        };

        // Step 3: Skip settlement if the handler returned an error response.
        if response.status().is_client_error() || response.status().is_server_error() {
            return Ok(response.into_response());
        }

        // Step 4: Settle and attach the Payment-Response header.
        let settlement = verified.settle(&self.facilitator).await?;
        let header_value = settlement_to_header(&settlement)?;

        let mut res = response;
        res.headers_mut().insert("Payment-Response", header_value);
        Ok(res.into_response())
    }
}

/// Extracts the payment header value from the header map.
pub fn extract_payment_header<'a>(
    header_map: &'a HeaderMap,
    header_name: &'a str,
) -> Option<&'a [u8]> {
    header_map.get(header_name).map(HeaderValue::as_bytes)
}

/// Extracts and deserializes the payment payload from base64-encoded header bytes.
#[must_use]
pub fn extract_payment_payload<T>(header_bytes: &[u8]) -> Option<T>
where
    T: serde::de::DeserializeOwned,
{
    let base64 = Base64Bytes::from(header_bytes).decode().ok()?;
    let value = serde_json::from_slice(base64.as_ref()).ok()?;
    Some(value)
}

/// Converts a **successful** [`proto::SettleResponse`] into an HTTP header value.
///
/// Delegates to [`proto::SettleResponse::encode_base64`] for the actual encoding,
/// which rejects error settlements at the type level.
///
/// # Errors
///
/// Returns [`PaygateError::Settlement`] if the response is an error variant
/// or if serialisation / header encoding fails.
pub fn settlement_to_header(
    settlement: &proto::SettleResponse,
) -> Result<HeaderValue, PaygateError> {
    let encoded = settlement
        .encode_base64()
        .ok_or_else(|| PaygateError::Settlement("cannot encode error settlement".to_owned()))?;
    HeaderValue::from_bytes(encoded.as_ref())
        .map_err(|err| PaygateError::Settlement(err.to_string()))
}

/// Constructs a V2 verify request from the payment payload and accepted requirements.
///
/// # Errors
///
/// Returns [`VerificationError::NoPaymentMatching`] if the accepted price tag is not found
/// in the accepts list, or a serialization error if the request cannot be converted to JSON.
pub fn make_verify_request(
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
///
/// # Errors
///
/// Returns [`VerificationError::VerificationFailed`] if the response is invalid or unknown.
pub fn validate_verify_response(
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
///
/// # Panics
///
/// Panics if the payment required response cannot be serialized to JSON or if the
/// HTTP response cannot be constructed. These conditions indicate a bug in the code.
pub fn error_into_response(
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
                .header("Content-Type", "application/json")
                .body(Body::from(payment_required_bytes))
                .expect("Fail to construct response")
        }
        PaygateError::Settlement(ref err) => {
            #[cfg(feature = "telemetry")]
            tracing::error!(details = %err, "Settlement failed");
            let body = Body::from(
                json!({ "error": "Settlement failed", "details": err.clone() }).to_string(),
            );
            Response::builder()
                .status(StatusCode::PAYMENT_REQUIRED)
                .header("Content-Type", "application/json")
                .body(body)
                .expect("Fail to construct response")
        }
    }
}
