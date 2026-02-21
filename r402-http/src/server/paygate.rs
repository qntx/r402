//! Core payment gate logic for enforcing x402 payments (V2-only).
//!
//! The [`Paygate`] struct handles the full payment lifecycle:
//! extracting headers, verifying with the facilitator, settling on-chain,
//! and returning 402 responses when payment is required.
//!
//! Three settlement strategies are available:
//!
//! - **Sequential** ([`Paygate::handle_request`]):
//!   verify → execute → settle. Settlement only runs after the handler
//!   succeeds.
//! - **Concurrent** ([`Paygate::handle_request_concurrent`]):
//!   verify → (settle ∥ execute) → await settle. Settlement runs in
//!   parallel with the handler, reducing total latency by one settle RTT.
//! - **Background** ([`Paygate::handle_request_background`]):
//!   verify → spawn settle (fire-and-forget) → execute → return. Ideal for
//!   streaming responses where the client should receive data immediately.

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

const PAYMENT_HEADER: &str = "Payment-Signature";

/// Verification errors for the payment gate.
#[derive(Debug, thiserror::Error)]
pub enum VerificationError {
    /// The `Payment-Signature` header is missing from the request.
    #[error("Payment-Signature header is required")]
    PaymentHeaderMissing,
    /// The payment header is present but malformed.
    #[error("Invalid or malformed payment header")]
    InvalidPaymentHeader,
    /// No accepted price tag matches the payment payload.
    #[error("Unable to find matching payment requirements")]
    NoPaymentMatching,
    /// The facilitator rejected the payment.
    #[error("Verification failed: {0}")]
    VerificationFailed(String),
}

/// Payment gate error encompassing verification and settlement failures.
#[derive(Debug, thiserror::Error)]
pub enum PaygateError {
    /// Payment verification failed.
    #[error(transparent)]
    Verification(#[from] VerificationError),
    /// On-chain settlement failed.
    #[error("Settlement failed: {0}")]
    Settlement(String),
}

type PaymentPayload = v2::PaymentPayload<v2::PaymentRequirements, serde_json::Value>;

/// Template for resource metadata included in 402 responses.
///
/// When `url` is `None`, the full resource URL is derived at request time
/// from the base URL and the request URI.
#[derive(Debug, Clone)]
pub struct ResourceTemplate {
    /// Description of the protected resource.
    pub description: String,
    /// MIME type of the protected resource.
    pub mime_type: String,
    /// Optional explicit URL; when `None`, derived from the request.
    pub url: Option<String>,
}

impl Default for ResourceTemplate {
    fn default() -> Self {
        Self {
            description: String::new(),
            mime_type: "application/json".to_owned(),
            url: None,
        }
    }
}

impl ResourceTemplate {
    /// Resolves this template into a concrete [`v2::ResourceInfo`].
    ///
    /// If `url` is already set, it is used directly. Otherwise, the URL is
    /// constructed by joining `base_url` (or a fallback derived from the
    /// `Host` header) with the request path and query.
    ///
    /// # Panics
    ///
    /// Panics if the hardcoded fallback URL `http://localhost` cannot be
    /// parsed, which should never happen in practice.
    #[allow(clippy::unwrap_used)]
    pub fn resolve(&self, base_url: Option<&Url>, req: &Request) -> v2::ResourceInfo {
        let url = self.url.clone().unwrap_or_else(|| {
            let mut url = base_url.cloned().unwrap_or_else(|| {
                let host = req
                    .headers()
                    .get("host")
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("localhost");
                let origin = format!("http://{host}");
                let url =
                    Url::parse(&origin).unwrap_or_else(|_| Url::parse("http://localhost").unwrap());
                #[cfg(feature = "telemetry")]
                tracing::warn!(
                    "X402Middleware base_url is not configured; \
                     using {url} as origin for resource resolution"
                );
                url
            });
            url.set_path(req.uri().path());
            url.set_query(req.uri().query());
            url.to_string()
        });
        v2::ResourceInfo {
            description: self.description.clone(),
            mime_type: self.mime_type.clone(),
            url,
        }
    }
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

    /// Converts a [`PaygateError`] into a proper HTTP response.
    ///
    /// Verification errors produce a 402 with the `Payment-Required` header
    /// and a JSON body. Settlement errors produce a 402 with error details.
    ///
    /// # Panics
    ///
    /// Panics if the payment-required response cannot be serialized to JSON
    /// or if the HTTP response builder fails. These indicate a bug.
    #[must_use]
    pub fn error_response(&self, err: PaygateError) -> Response {
        match err {
            PaygateError::Verification(ve) => {
                let payment_required = v2::PaymentRequired {
                    error: Some(ve.to_string()),
                    accepts: self
                        .accepts
                        .iter()
                        .map(|pt| pt.requirements.clone())
                        .collect(),
                    x402_version: v2::V2,
                    resource: self.resource.clone(),
                    extensions: None,
                };
                let body_bytes =
                    serde_json::to_vec(&payment_required).expect("serialization failed");
                let header_value =
                    HeaderValue::from_bytes(Base64Bytes::encode(&body_bytes).as_ref())
                        .expect("invalid header value");

                Response::builder()
                    .status(StatusCode::PAYMENT_REQUIRED)
                    .header("Payment-Required", header_value)
                    .header("Content-Type", "application/json")
                    .body(Body::from(body_bytes))
                    .expect("failed to construct response")
            }
            PaygateError::Settlement(ref detail) => {
                #[cfg(feature = "telemetry")]
                tracing::error!(details = %detail, "Settlement failed");
                let body = json!({ "error": "Settlement failed", "details": detail }).to_string();

                Response::builder()
                    .status(StatusCode::PAYMENT_REQUIRED)
                    .header("Content-Type", "application/json")
                    .body(Body::from(body))
                    .expect("failed to construct response")
            }
        }
    }
}

impl<TFacilitator> Paygate<TFacilitator>
where
    TFacilitator: Facilitator + Sync,
{
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
            .collect();
        self.accepts = Arc::new(accepts);
    }

    /// Verifies the payment from request headers without executing the inner
    /// service or settling on-chain.
    ///
    /// Returns a [`VerifiedPayment`] token on success, which the caller can
    /// later [`settle`](VerifiedPayment::settle) at their discretion.
    ///
    /// # Errors
    ///
    /// Returns [`PaygateError::Verification`] if the payment header is missing,
    /// malformed, or rejected by the facilitator.
    #[cfg_attr(feature = "telemetry", instrument(name = "x402.verify_only", skip_all))]
    pub async fn verify_only(&self, headers: &HeaderMap) -> Result<VerifiedPayment, PaygateError> {
        let header_bytes = headers
            .get(PAYMENT_HEADER)
            .map(HeaderValue::as_bytes)
            .ok_or(VerificationError::PaymentHeaderMissing)?;

        let payload: PaymentPayload =
            decode_payment_payload(header_bytes).ok_or(VerificationError::InvalidPaymentHeader)?;

        let verify_request = build_verify_request(payload, &self.accepts)?;

        let verify_response = self
            .facilitator
            .verify(verify_request.clone())
            .await
            .map_err(|e| VerificationError::VerificationFailed(format!("{e}")))?;

        if let proto::VerifyResponse::Invalid { reason, .. } = verify_response {
            return Err(VerificationError::VerificationFailed(reason).into());
        }

        Ok(VerifiedPayment {
            settle_request: verify_request.into(),
        })
    }

    /// Handles an incoming request with **sequential** settlement.
    ///
    /// ```text
    /// verify → execute → settle → attach header → return
    /// ```
    ///
    /// Settlement only runs if the handler returns a success status (not 4xx/5xx).
    ///
    /// # Errors
    ///
    /// Returns [`PaygateError`] if payment verification or settlement fails.
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
    ) -> Result<Response, PaygateError>
    where
        S::Response: IntoResponse,
        S::Error: IntoResponse,
        S::Future: Send,
    {
        let verified = self.verify_only(req.headers()).await?;

        let response = match call_inner(inner, req).await {
            Ok(r) => r,
            Err(err) => return Ok(err.into_response()),
        };

        if response.status().is_client_error() || response.status().is_server_error() {
            return Ok(response.into_response());
        }

        let settlement = verified.settle(&self.facilitator).await?;
        let header_value = settlement_to_header(&settlement)?;

        let mut res = response;
        res.headers_mut().insert("Payment-Response", header_value);
        Ok(res.into_response())
    }
}

impl<TFacilitator> Paygate<TFacilitator>
where
    TFacilitator: Facilitator + Clone + Send + Sync + 'static,
{
    /// Handles an incoming request with **concurrent** settlement.
    ///
    /// ```text
    /// verify → (settle ∥ execute) → await settle → attach header → return
    /// ```
    ///
    /// Settlement is spawned immediately after verification and runs in
    /// parallel with the handler, reducing total latency by one facilitator RTT.
    /// On handler error (4xx/5xx), the settlement task is abandoned.
    ///
    /// # Errors
    ///
    /// Returns [`PaygateError`] if payment verification or settlement fails.
    #[cfg_attr(
        feature = "telemetry",
        instrument(name = "x402.handle_request_concurrent", skip_all)
    )]
    pub async fn handle_request_concurrent<
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
        S::Future: Send + 'static,
        ReqBody: Send + 'static,
    {
        let verified = self.verify_only(req.headers()).await?;

        let facilitator = self.facilitator.clone();
        let settle_handle = tokio::spawn(async move { verified.settle(&facilitator).await });

        let response = match call_inner(inner, req).await {
            Ok(r) => r,
            Err(err) => {
                drop(settle_handle);
                return Ok(err.into_response());
            }
        };

        if response.status().is_client_error() || response.status().is_server_error() {
            drop(settle_handle);
            return Ok(response.into_response());
        }

        let settlement = settle_handle
            .await
            .map_err(|e| PaygateError::Settlement(format!("settle task panicked: {e}")))??;
        let header_value = settlement_to_header(&settlement)?;

        let mut res = response;
        res.headers_mut().insert("Payment-Response", header_value);
        Ok(res.into_response())
    }

    /// Handles an incoming request with **background** (fire-and-forget) settlement.
    ///
    /// ```text
    /// verify → spawn settle (fire-and-forget) → execute → return
    /// ```
    ///
    /// Settlement is spawned immediately after verification but **never awaited**.
    /// The response is returned to the client as soon as the handler completes,
    /// without waiting for on-chain settlement.
    ///
    /// This is ideal for **streaming** responses (e.g. SSE / LLM token streams)
    /// where the client should start receiving data immediately.
    ///
    /// **Trade-off:** the `Payment-Response` header is **not** attached to the
    /// response since settlement may still be in progress.
    ///
    /// # Errors
    ///
    /// Returns [`PaygateError::Verification`] if payment verification fails.
    /// Settlement errors are logged but do not propagate.
    #[cfg_attr(
        feature = "telemetry",
        instrument(name = "x402.handle_request_background", skip_all)
    )]
    pub async fn handle_request_background<
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
        S::Future: Send + 'static,
        ReqBody: Send + 'static,
    {
        let verified = self.verify_only(req.headers()).await?;

        // Fire-and-forget: spawn settlement without awaiting
        let facilitator = self.facilitator.clone();
        tokio::spawn(async move {
            if let Err(e) = verified.settle(&facilitator).await {
                #[cfg(feature = "telemetry")]
                tracing::error!(error = %e, "background settlement failed");
                #[cfg(not(feature = "telemetry"))]
                let _ = e;
            }
        });

        match call_inner(inner, req).await {
            Ok(r) => Ok(r.into_response()),
            Err(err) => Ok(err.into_response()),
        }
    }
}

/// A verified payment token ready for on-chain settlement.
///
/// Produced by [`Paygate::verify_only`] after the facilitator confirms the
/// payment signature is valid. [`settle`](Self::settle) **consumes** `self`,
/// preventing double-settlement at the type level.
#[derive(Debug)]
pub struct VerifiedPayment {
    settle_request: proto::SettleRequest,
}

impl VerifiedPayment {
    /// Executes on-chain settlement, consuming `self` to prevent reuse.
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

/// Encodes a successful [`proto::SettleResponse`] as an HTTP header value.
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
    HeaderValue::from_bytes(encoded.as_ref()).map_err(|e| PaygateError::Settlement(e.to_string()))
}

/// Calls the inner service with optional telemetry instrumentation.
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

/// Decodes a base64-encoded JSON payment payload from raw header bytes.
fn decode_payment_payload<T: serde::de::DeserializeOwned>(header_bytes: &[u8]) -> Option<T> {
    let decoded = Base64Bytes::from(header_bytes).decode().ok()?;
    serde_json::from_slice(decoded.as_ref()).ok()
}

/// Matches the payment payload against accepted price tags and builds a
/// [`proto::VerifyRequest`].
fn build_verify_request(
    payload: PaymentPayload,
    accepts: &[v2::PriceTag],
) -> Result<proto::VerifyRequest, VerificationError> {
    let selected = accepts
        .iter()
        .find(|pt| **pt == payload.accepted)
        .ok_or(VerificationError::NoPaymentMatching)?;

    let verify = v2::VerifyRequest {
        x402_version: v2::V2,
        payment_payload: payload,
        payment_requirements: selected.requirements.clone(),
    };

    let json = serde_json::to_value(&verify)
        .map_err(|e| VerificationError::VerificationFailed(format!("{e}")))?;

    Ok(proto::VerifyRequest::from(json))
}
