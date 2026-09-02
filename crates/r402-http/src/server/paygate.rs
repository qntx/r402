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
use http::header::ACCESS_CONTROL_EXPOSE_HEADERS;
use http::{HeaderMap, HeaderValue, StatusCode};
use r402_core::error::ErrorReason;
use r402_core::facilitator::{DynFacilitator, Facilitator};
use r402_core::resource_server::{CancelReason, ResourceServer, ResourceServerHooks};
use r402_core::wire;
use r402_core::wire::Base64Bytes;
use serde_json::json;
use tower::Service;
#[cfg(feature = "telemetry")]
use tracing::{Instrument, instrument};
use url::Url;

use super::hooks::DynPaygateHooks;
use super::tracker::{BackgroundSettlementTracker, SettlementInFlightGuard};

const PAYMENT_HEADER: &str = "Payment-Signature";

/// The canonical list of x402 response headers clients need to see.
pub const X402_EXPOSED_HEADERS: &str = "Payment-Required, Payment-Response";

/// Ensures `Access-Control-Expose-Headers` advertises the x402 response
/// headers. Idempotent.
pub fn ensure_expose_headers(headers: &mut HeaderMap) {
    let x402 = HeaderValue::from_static(X402_EXPOSED_HEADERS);
    match headers.get(ACCESS_CONTROL_EXPOSE_HEADERS) {
        None => {
            let _ = headers.insert(ACCESS_CONTROL_EXPOSE_HEADERS, x402);
        }
        Some(existing) => {
            let Ok(existing_str) = existing.to_str() else {
                let _ = headers.insert(ACCESS_CONTROL_EXPOSE_HEADERS, x402);
                return;
            };
            if existing_str.contains("Payment-Required")
                && existing_str.contains("Payment-Response")
            {
                return;
            }
            let merged = format!("{existing_str}, {X402_EXPOSED_HEADERS}");
            if let Ok(value) = HeaderValue::from_str(&merged) {
                let _ = headers.insert(ACCESS_CONTROL_EXPOSE_HEADERS, value);
            }
        }
    }
}

/// Returns the HTTP status corresponding to an [`ErrorReason`].
///
/// `Permit2AllowanceRequired` maps to `412`; everything else to `402`.
#[must_use]
pub const fn reason_to_status(reason: &ErrorReason) -> StatusCode {
    match reason {
        ErrorReason::Permit2AllowanceRequired => StatusCode::PRECONDITION_FAILED,
        _ => StatusCode::PAYMENT_REQUIRED,
    }
}

/// Payment gate error encompassing header, verification, and settlement failures.
#[derive(Debug, thiserror::Error)]
pub enum PaygateError {
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
    /// Facilitator returned a structured `SettleResponse::Failure`.
    ///
    /// The failure body is preserved end-to-end so the paygate can emit it
    /// via the `Payment-Response` HTTP header per x402 v2 §HTTP transport,
    /// giving browser clients access to the machine-readable error reason.
    #[error("settlement failed: {}", settlement_failure_summary(.0))]
    Settlement(Box<wire::SettleResponse>),
    /// Internal error before a structured settlement response could be
    /// obtained (timeout, panic in spawned task, malformed override, etc.).
    /// Renders as a 402 with no `Payment-Response` header.
    #[error("settlement aborted: {0}")]
    SettlementAborted(String),
}

#[allow(
    clippy::missing_const_for_fn,
    reason = "const fn would prevent matching on `Box` indirection"
)]
fn settlement_failure_summary(resp: &wire::SettleResponse) -> String {
    match resp {
        wire::SettleResponse::Failure {
            reason,
            message,
            network,
            ..
        } => format!(
            "{} ({}){}",
            reason,
            network,
            message
                .as_ref()
                .map(|m| format!(": {m}"))
                .unwrap_or_default(),
        ),
        wire::SettleResponse::Success { .. } => "success returned via error path".to_owned(),
        // wire::SettleResponse is `#[non_exhaustive]`; future variants
        // surface as a generic placeholder so the formatter remains total.
        _ => "unknown settlement variant".to_owned(),
    }
}

type PaymentPayload = wire::PaymentPayload<wire::PaymentRequirements, serde_json::Value>;

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
    /// Resolves this template into a concrete [`wire::ResourceInfo`].
    ///
    /// If `url` is already set, it is used directly. Otherwise, the URL is
    /// constructed by joining `base_url` (or a fallback derived from the
    /// `Host` header) with the request path and query.
    ///
    /// # Panics
    ///
    /// Panics if the hardcoded fallback URL `http://localhost` cannot be
    /// parsed, which should never happen in practice.
    #[allow(clippy::unwrap_used, reason = "fallback URL is a hardcoded constant")]
    pub fn resolve(&self, base_url: Option<&Url>, req: &Request) -> wire::ResourceInfo {
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
        let mut info = wire::ResourceInfo::new(url);
        if !self.description.is_empty() {
            info = info.with_description(self.description.clone());
        }
        if !self.mime_type.is_empty() {
            info = info.with_mime_type(self.mime_type.clone());
        }
        info
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
#[allow(
    missing_debug_implementations,
    reason = "ResourceServer contains dyn facilitator handles"
)]
pub struct PaygateBuilder {
    server: ResourceServer,
    accepts: Vec<wire::PriceTag>,
    resource: Option<wire::ResourceInfo>,
    hooks: Option<Arc<dyn DynPaygateHooks>>,
    settlement_tracker: Option<BackgroundSettlementTracker>,
}

impl PaygateBuilder {
    /// Adds a single accepted payment option.
    #[must_use]
    pub fn accept(mut self, price_tag: wire::PriceTag) -> Self {
        self.accepts.push(price_tag);
        self
    }

    /// Adds multiple accepted payment options.
    #[must_use]
    pub fn accepts(mut self, price_tags: impl IntoIterator<Item = wire::PriceTag>) -> Self {
        self.accepts.extend(price_tags);
        self
    }

    /// Sets the resource metadata returned in 402 responses.
    #[must_use]
    pub fn resource(mut self, resource: wire::ResourceInfo) -> Self {
        self.resource = Some(resource);
        self
    }

    /// Attaches [`PaygateHooks`](super::hooks::PaygateHooks) for pre- and
    /// post-payment extensibility (Fix-8). Stored as an `Arc<dyn>` so hook
    /// state can be shared across cloned middleware instances without
    /// duplication.
    #[must_use]
    pub fn hooks<H>(mut self, hooks: H) -> Self
    where
        H: super::hooks::PaygateHooks + 'static,
    {
        self.hooks = Some(Arc::new(hooks));
        self
    }

    /// Attaches hooks that are already stored behind an
    /// [`Arc<dyn DynPaygateHooks>`].
    ///
    /// This avoids re-wrapping when the same hook object needs to be shared
    /// between the middleware layer and the paygate it constructs.
    #[must_use]
    pub fn hooks_dyn(mut self, hooks: Arc<dyn DynPaygateHooks>) -> Self {
        self.hooks = Some(hooks);
        self
    }

    /// Registers a transport-agnostic [`ResourceServerHooks`] lifecycle hook.
    #[must_use]
    pub fn with_resource_hook(mut self, hook: impl ResourceServerHooks + 'static) -> Self {
        self.server.add_hook(hook);
        self
    }

    /// Attaches a [`BackgroundSettlementTracker`] so background settlement
    /// tasks register with it. Used to await in-flight settlements during
    /// graceful shutdown via [`Paygate::settlement_tracker`] +
    /// [`BackgroundSettlementTracker::wait_for_drain`].
    #[must_use]
    pub fn with_settlement_tracker(mut self, tracker: BackgroundSettlementTracker) -> Self {
        self.settlement_tracker = Some(tracker);
        self
    }

    /// Consumes the builder and produces a configured [`Paygate`].
    ///
    /// Uses empty resource info if none was provided.
    #[must_use]
    pub fn build(self) -> Paygate {
        Paygate {
            server: self.server,
            accepts: self.accepts.into(),
            resource: self
                .resource
                .unwrap_or_else(|| wire::ResourceInfo::new("").with_mime_type("application/json")),
            hooks: self.hooks,
            settlement_tracker: self.settlement_tracker,
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
/// facilitator with [`HookedFacilitator`](r402_core::facilitator::HookedFacilitator)
/// before passing it to the payment gate.
#[allow(
    missing_debug_implementations,
    reason = "ResourceServer contains dyn facilitator handles"
)]
pub struct Paygate {
    pub(crate) server: ResourceServer,
    pub(crate) accepts: Arc<[wire::PriceTag]>,
    pub(crate) resource: wire::ResourceInfo,
    pub(crate) hooks: Option<Arc<dyn DynPaygateHooks>>,
    /// Optional tracker for background settlement tasks.
    ///
    /// When [`PaygateBuilder::with_settlement_tracker`] is set, every
    /// `handle_request_background` call increments the in-flight counter
    /// before spawning and decrements it once the supervisor records the
    /// outcome. Operators call [`Self::settlement_tracker`] +
    /// [`BackgroundSettlementTracker::wait_for_drain`] during shutdown to
    /// await the drain (with a timeout safeguard).
    pub(crate) settlement_tracker: Option<BackgroundSettlementTracker>,
}

impl Paygate {
    /// Returns a new builder seeded with the given facilitator.
    pub fn builder(facilitator: impl Facilitator + 'static) -> PaygateBuilder {
        PaygateBuilder {
            server: ResourceServer::new(Arc::new(facilitator)),
            accepts: Vec::new(),
            resource: None,
            hooks: None,
            settlement_tracker: None,
        }
    }

    /// Returns a new builder over an already-erased facilitator handle.
    #[must_use]
    pub fn builder_from_dyn(facilitator: Arc<dyn DynFacilitator>) -> PaygateBuilder {
        PaygateBuilder {
            server: ResourceServer::from_dyn(facilitator),
            accepts: Vec::new(),
            resource: None,
            hooks: None,
            settlement_tracker: None,
        }
    }

    /// Returns a new builder from an existing [`ResourceServer`] (hooks included).
    #[must_use]
    pub fn builder_from_server(server: ResourceServer) -> PaygateBuilder {
        PaygateBuilder {
            server,
            accepts: Vec::new(),
            resource: None,
            hooks: None,
            settlement_tracker: None,
        }
    }

    /// Returns a reference to the resource server (facilitator + hooks).
    #[must_use]
    pub const fn resource_server(&self) -> &ResourceServer {
        &self.server
    }

    /// Returns a clone of the erased facilitator handle.
    #[must_use]
    pub fn facilitator(&self) -> Arc<dyn DynFacilitator> {
        self.server.facilitator()
    }

    /// Returns a reference to the accepted price tags.
    #[must_use]
    pub fn accepts(&self) -> &[wire::PriceTag] {
        &self.accepts
    }

    /// Returns the in-flight settlement tracker, if one was attached at
    /// construction time.
    ///
    /// The handle is shareable; clone it and pass to a shutdown task to
    /// await the in-flight drain via
    /// [`BackgroundSettlementTracker::wait_for_drain`]:
    ///
    /// ```ignore
    /// if let Some(tracker) = paygate.settlement_tracker().cloned() {
    ///     tokio::spawn(async move {
    ///         match tracker.wait_for_drain(Duration::from_secs(30)).await {
    ///             Ok(()) => tracing::info!("settle drain complete"),
    ///             Err(remaining) => tracing::warn!(remaining, "drain timeout"),
    ///         }
    ///     });
    /// }
    /// ```
    #[must_use]
    pub const fn settlement_tracker(&self) -> Option<&BackgroundSettlementTracker> {
        self.settlement_tracker.as_ref()
    }

    /// Returns a reference to the resource information.
    #[must_use]
    pub const fn resource(&self) -> &wire::ResourceInfo {
        &self.resource
    }

    /// Returns the attached paygate hooks, if any.
    ///
    /// The middleware layer uses this accessor to dispatch
    /// [`DynPaygateHooks::on_protected_request`] and
    /// [`DynPaygateHooks::on_payment_verified`] around the payment check.
    #[must_use]
    pub fn hooks(&self) -> Option<&Arc<dyn DynPaygateHooks>> {
        self.hooks.as_ref()
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
    #[allow(
        clippy::expect_used,
        reason = "infallible JSON/HTTP construction; panic indicates a bug"
    )]
    pub fn error_response(&self, err: PaygateError) -> Response {
        match err {
            PaygateError::PaymentHeaderMissing
            | PaygateError::InvalidPaymentHeader
            | PaygateError::NoPaymentMatching
            | PaygateError::VerificationFailed(_) => {
                let (status, payment_required) = {
                    let status = inferred_status(&err);
                    let payment_required = wire::PaymentRequired::new(self.resource.clone())
                        .with_error(err.to_string())
                        .with_accepts(
                            self.accepts
                                .iter()
                                .map(|pt| pt.requirements.clone())
                                .collect(),
                        );
                    (status, payment_required)
                };
                let body_bytes =
                    serde_json::to_vec(&payment_required).expect("serialization failed");
                let header_value =
                    HeaderValue::from_bytes(Base64Bytes::encode(&body_bytes).as_ref())
                        .expect("invalid header value");

                let mut response = Response::builder()
                    .status(status)
                    .header("Payment-Required", header_value)
                    .header("Content-Type", "application/json")
                    .body(Body::from(body_bytes))
                    .expect("failed to construct response");
                // Fix-6: expose Payment-Required / Payment-Response headers to
                // browser clients via CORS.
                ensure_expose_headers(response.headers_mut());
                response
            }
            PaygateError::Settlement(failure) => {
                #[cfg(feature = "telemetry")]
                tracing::error!(failure = ?failure, "Settlement failed");
                let body_bytes = serde_json::to_vec(&*failure).expect("serialization failed");
                let header_value = failure
                    .encode_base64_any()
                    .and_then(|b64| HeaderValue::from_bytes(b64.as_ref()).ok());

                let mut builder = Response::builder()
                    .status(StatusCode::PAYMENT_REQUIRED)
                    .header("Content-Type", "application/json");
                if let Some(header_value) = header_value {
                    builder = builder.header("Payment-Response", header_value);
                }
                let mut response = builder
                    .body(Body::from(body_bytes))
                    .expect("failed to construct response");
                ensure_expose_headers(response.headers_mut());
                response
            }
            PaygateError::SettlementAborted(ref detail) => {
                #[cfg(feature = "telemetry")]
                tracing::error!(details = %detail, "Settlement aborted");
                let body = json!({
                    "error": "settlement aborted",
                    "details": detail,
                })
                .to_string();

                let mut response = Response::builder()
                    .status(StatusCode::PAYMENT_REQUIRED)
                    .header("Content-Type", "application/json")
                    .body(Body::from(body))
                    .expect("failed to construct response");
                ensure_expose_headers(response.headers_mut());
                response
            }
        }
    }
}

impl Paygate {
    /// Enriches price tags with facilitator capabilities (e.g., fee payer address).
    pub async fn enrich_accepts(&mut self) {
        let facilitator = self.facilitator();
        let capabilities = Facilitator::supported(&facilitator)
            .await
            .unwrap_or_default();
        let accepts: Vec<_> = self
            .accepts
            .iter()
            .cloned()
            .map(|mut pt| {
                pt.enrich(&capabilities);
                pt
            })
            .collect();
        self.accepts = accepts.into();
    }

    /// Verifies the payment from request headers without executing the inner
    /// service or settling on-chain.
    ///
    /// Runs [`ResourceServer`] lifecycle hooks (before/after verify, failure
    /// recovery). Returns a [`VerifiedPayment`] token on success.
    ///
    /// # Errors
    ///
    /// Returns a verification [`PaygateError`] if the payment header is missing,
    /// malformed, or rejected by the facilitator / hooks.
    #[cfg_attr(feature = "telemetry", instrument(name = "x402.verify_only", skip_all))]
    pub async fn verify_only(&self, headers: &HeaderMap) -> Result<VerifiedPayment, PaygateError> {
        let header_bytes = headers
            .get(PAYMENT_HEADER)
            .map(HeaderValue::as_bytes)
            .ok_or(PaygateError::PaymentHeaderMissing)?;

        let payload: PaymentPayload =
            decode_payment_payload(header_bytes).ok_or(PaygateError::InvalidPaymentHeader)?;

        let available: Vec<wire::PaymentRequirements> = self
            .accepts
            .iter()
            .map(|pt| pt.requirements.clone())
            .collect();
        let requirements = self
            .server
            .find_matching_requirements(&available, &payload)
            .cloned()
            .ok_or(PaygateError::NoPaymentMatching)?;
        let outcome = self
            .server
            .verify_payment(&payload, &requirements)
            .await
            .map_err(|e| PaygateError::VerificationFailed(format!("{e}")))?;

        if let wire::VerifyResponse::Invalid { reason, .. } = &outcome.response {
            return Err(PaygateError::VerificationFailed(reason.to_string()));
        }

        // Build settle request from the same payload/requirements pair.
        let settle_request = build_settle_request(&payload, &requirements).map_err(|e| {
            PaygateError::VerificationFailed(format!("settle request build failed: {e}"))
        })?;

        Ok(VerifiedPayment {
            settle_request,
            payload,
            requirements,
            server: self.server.clone(),
            skip_handler: outcome.skip_handler,
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
        let cancel = verified.cancellation_guard();

        // After-verify SkipHandler: settle without invoking the resource handler.
        if let Some(directive) = verified.skip_handler.clone() {
            let settlement = verified.settle_with_override(None).await?;
            return skip_handler_response(&directive, &settlement);
        }

        let response = match call_inner(inner, req).await {
            Ok(r) => r,
            Err(err) => {
                cancel
                    .cancel(
                        CancelReason::HandlerThrew,
                        Some("inner service error"),
                        None,
                    )
                    .await;
                return Ok(err.into_response());
            }
        };

        if response.status().is_client_error() || response.status().is_server_error() {
            cancel
                .cancel(
                    CancelReason::HandlerFailed,
                    Some("handler returned error status"),
                    Some(response.status().as_u16()),
                )
                .await;
            return Ok(response.into_response());
        }

        let mut response = response.into_response();
        // Upto: Settlement-Overrides header and/or UptoActualAmount extension.
        let override_amount =
            super::upto::resolve_response_settlement_amount(&mut response, verified.requirements())
                .map_err(|e| PaygateError::SettlementAborted(e.to_string()))?;

        let settlement = verified
            .settle_with_override(override_amount.as_deref())
            .await?;
        let header_value = settlement_to_header(&settlement)?;

        response
            .headers_mut()
            .insert("Payment-Response", header_value);
        // Browser clients need Access-Control-Expose-Headers for Payment-Response.
        ensure_expose_headers(response.headers_mut());
        Ok(response)
    }
}

impl Paygate {
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
        let cancel = verified.cancellation_guard();

        if let Some(directive) = verified.skip_handler.clone() {
            let settlement = verified.settle().await?;
            return skip_handler_response(&directive, &settlement);
        }

        // Concurrent settle runs at the signed maximum in parallel with the
        // handler. Partial settlement (Settlement-Overrides / UptoActualAmount)
        // requires Sequential mode — reject rather than silently over-charge.
        let settle_handle = tokio::spawn(async move { verified.settle().await });

        let response = match call_inner(inner, req).await {
            Ok(r) => r,
            Err(err) => {
                drop(settle_handle);
                cancel
                    .cancel(
                        CancelReason::HandlerThrew,
                        Some("inner service error"),
                        None,
                    )
                    .await;
                return Ok(err.into_response());
            }
        };

        if response.status().is_client_error() || response.status().is_server_error() {
            drop(settle_handle);
            cancel
                .cancel(
                    CancelReason::HandlerFailed,
                    Some("handler returned error status"),
                    Some(response.status().as_u16()),
                )
                .await;
            return Ok(response.into_response());
        }

        let mut res = response.into_response();
        // Strip billing header even when unused so it never reaches the client.
        let partial = super::upto::take_settlement_overrides_header(res.headers_mut());
        let has_ext = res
            .extensions_mut()
            .remove::<super::upto::UptoActualAmount>()
            .is_some();
        if partial.is_some() || has_ext {
            drop(settle_handle);
            return Err(PaygateError::SettlementAborted(
                "Settlement-Overrides / UptoActualAmount require SettlementMode::Sequential".into(),
            ));
        }

        let settlement = settle_handle
            .await
            .map_err(|e| PaygateError::SettlementAborted(format!("settle task panicked: {e}")))??;
        let header_value = settlement_to_header(&settlement)?;

        res.headers_mut().insert("Payment-Response", header_value);
        ensure_expose_headers(res.headers_mut());
        Ok(res)
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
    /// Returns a verification [`PaygateError`] if payment verification fails.
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
        let cancel = verified.cancellation_guard();

        // SkipHandler: no resource body to stream — settle inline and return
        // the directive response (same as sequential/concurrent).
        if let Some(directive) = verified.skip_handler.clone() {
            let settlement = verified.settle().await?;
            return skip_handler_response(&directive, &settlement);
        }

        // F-103: spawn the settlement task and a supervisor that awaits the
        // join handle. The supervisor surfaces three failure modes that
        // would otherwise be silenced:
        //
        // - structured `FacilitatorError` from `settle()`,
        // - panics inside the settle task (lost into the void by tokio),
        // - cancellations (e.g. runtime shutdown).
        //
        // Two `tokio::spawn` calls cost a single extra heap allocation per
        // request — negligible compared to the on-chain work — and we get
        // observable settlement outcomes in exchange.
        //
        // Partial settlement is not available in background mode (settle
        // starts at the signed maximum before the handler returns).
        let settle_handle = tokio::spawn(async move { verified.settle().await });
        // F-101/F-102: register with the optional tracker before spawning
        // the supervisor so `wait_for_pending_settlements` observes the
        // task even if the supervisor finishes within microseconds.
        let tracker_guard = self
            .settlement_tracker
            .as_ref()
            .map(BackgroundSettlementTracker::start);
        // Detached supervisor: we deliberately drop the JoinHandle. The
        // supervisor itself never panics and only logs, so leaking the
        // handle is the cheapest fire-and-forget pattern.
        drop(tokio::spawn(supervise_background_settle(
            settle_handle,
            tracker_guard,
        )));

        // Bind the future result before matching so `cancel` outlives the
        // temporary (Rust 2024 tail-expression drop order).
        let call_result = call_inner(inner, req).await;
        match call_result {
            Ok(r) => {
                if r.status().is_client_error() || r.status().is_server_error() {
                    cancel
                        .cancel(
                            CancelReason::HandlerFailed,
                            Some("handler returned error status"),
                            Some(r.status().as_u16()),
                        )
                        .await;
                }
                let mut response = r.into_response();
                // Never leak internal billing headers to the client.
                drop(super::upto::take_settlement_overrides_header(
                    response.headers_mut(),
                ));
                drop(
                    response
                        .extensions_mut()
                        .remove::<super::upto::UptoActualAmount>(),
                );
                Ok(response)
            }
            Err(err) => {
                cancel
                    .cancel(
                        CancelReason::HandlerThrew,
                        Some("inner service error"),
                        None,
                    )
                    .await;
                Ok(err.into_response())
            }
        }
    }
}

/// A verified payment token ready for on-chain settlement.
///
/// Produced by [`Paygate::verify_only`] after the resource server confirms the
/// payment. [`settle`](Self::settle) **consumes** `self`, preventing
/// double-settlement at the type level.
#[derive(Debug)]
pub struct VerifiedPayment {
    settle_request: wire::SettleRequest,
    payload: PaymentPayload,
    requirements: wire::PaymentRequirements,
    server: ResourceServer,
    /// When set by after-verify hooks, the resource handler should be skipped.
    pub skip_handler: Option<r402_core::SkipHandlerDirective>,
}

impl VerifiedPayment {
    /// One-shot cancel dispatcher for this verified payment.
    #[must_use]
    pub fn cancellation_guard(&self) -> r402_core::CancellationGuard {
        self.server
            .cancellation_guard(self.payload.clone(), self.requirements.clone())
    }

    /// Executes on-chain settlement via the resource server, consuming `self`.
    ///
    /// # Errors
    ///
    /// Returns [`PaygateError::Settlement`] if settlement fails.
    pub async fn settle(self) -> Result<wire::SettleResponse, PaygateError> {
        self.settle_with_override(None).await
    }

    /// Like [`settle`](Self::settle) but applies an optional atomic amount
    /// override through [`ResourceServer::settle_payment`] (hooks included).
    ///
    /// Intended for the **upto** scheme after resolving
    /// [`Settlement-Overrides`](super::SETTLEMENT_OVERRIDES_HEADER) /
    /// [`UptoActualAmount`](super::UptoActualAmount).
    /// Passing `None` is equivalent to [`settle`](Self::settle).
    ///
    /// # Errors
    ///
    /// Returns [`PaygateError::Settlement`] when the override is rejected, the
    /// facilitator fails settlement, or settle hooks abort.
    pub async fn settle_with_override(
        self,
        actual_amount: Option<&str>,
    ) -> Result<wire::SettleResponse, PaygateError> {
        use r402_core::SettlementOverrides;

        let overrides = actual_amount.map(SettlementOverrides::amount);
        let settlement = self
            .server
            .settle_payment(&self.payload, &self.requirements, overrides.as_ref())
            .await
            .map_err(|e| PaygateError::SettlementAborted(format!("{e}")))?;

        if matches!(settlement, wire::SettleResponse::Failure { .. }) {
            return Err(PaygateError::Settlement(Box::new(settlement)));
        }

        Ok(settlement)
    }

    /// Matched payment requirements (for override resolution).
    #[must_use]
    pub const fn requirements(&self) -> &wire::PaymentRequirements {
        &self.requirements
    }

    /// Returns a reference to the underlying settle request.
    #[must_use]
    pub const fn settle_request(&self) -> &wire::SettleRequest {
        &self.settle_request
    }
}

/// Awaits the join handle of a background settlement task and surfaces the
/// outcome via tracing.
///
/// Three classes of failure are otherwise lost when a fire-and-forget
/// `tokio::spawn` is used directly:
///
/// 1. structured [`FacilitatorError`] returned by `settle()`,
/// 2. **panics** inside the spawn (tokio aborts the task but the host
///    process never sees the error),
/// 3. cancellations (e.g. runtime shutdown).
///
/// This supervisor logs each at the appropriate level so operators can
/// detect silent settlement failures in production. Telemetry is
/// feature-gated; without `telemetry` the supervisor still consumes the
/// outcome (preventing a panic-on-drop scenario for the `JoinHandle`).
async fn supervise_background_settle(
    handle: tokio::task::JoinHandle<Result<wire::SettleResponse, PaygateError>>,
    // Held until the supervisor finishes; on drop it decrements the
    // in-flight counter on the tracker (if any). We deliberately accept
    // the guard by value so the awaiting `wait_for_pending_settlements`
    // sees the task as in-flight until the supervisor logs its outcome.
    _tracker: Option<SettlementInFlightGuard>,
) {
    let outcome = handle.await;
    log_background_settle_outcome(outcome);
}

/// Logs the result of a background settlement task at the appropriate
/// level. Split out from [`supervise_background_settle`] so the supervisor
/// stays under clippy's cognitive-complexity limit and the logging
/// behaviour is unit-testable in isolation.
fn log_background_settle_outcome(
    outcome: Result<Result<wire::SettleResponse, PaygateError>, tokio::task::JoinError>,
) {
    match outcome {
        Ok(Ok(_settlement)) => {
            #[cfg(feature = "telemetry")]
            tracing::debug!("background settlement completed");
            record_background_settle_metric("ok");
        }
        Ok(Err(err)) => {
            log_background_settle_error(&err);
            record_background_settle_metric("error");
        }
        Err(join_err) => {
            let label = if join_err.is_panic() {
                "panic"
            } else {
                "cancelled"
            };
            log_background_settle_join_error(&join_err);
            record_background_settle_metric(label);
        }
    }
}

#[cfg(feature = "metrics")]
fn record_background_settle_metric(result: &'static str) {
    ::metrics::counter!(
        r402_core::metrics::PAYGATE_BACKGROUND_SETTLE_TOTAL,
        "result" => result,
    )
    .increment(1);
}
#[cfg(not(feature = "metrics"))]
fn record_background_settle_metric(_result: &'static str) {}

#[cfg(feature = "telemetry")]
fn log_background_settle_error(err: &PaygateError) {
    tracing::error!(error = %err, "background settlement returned error");
}
#[cfg(not(feature = "telemetry"))]
fn log_background_settle_error(_err: &PaygateError) {}

#[cfg(feature = "telemetry")]
fn log_background_settle_join_error(join_err: &tokio::task::JoinError) {
    if join_err.is_panic() {
        tracing::error!(error = %join_err, "background settlement task panicked");
    } else {
        tracing::warn!(error = %join_err, "background settlement task cancelled");
    }
}
#[cfg(not(feature = "telemetry"))]
fn log_background_settle_join_error(_join_err: &tokio::task::JoinError) {}

/// Encodes a successful [`wire::SettleResponse`] as an HTTP header value.
///
/// # Errors
///
/// Returns [`PaygateError::Settlement`] if the response is an error variant
/// or if serialisation / header encoding fails.
pub fn settlement_to_header(
    settlement: &wire::SettleResponse,
) -> Result<HeaderValue, PaygateError> {
    let encoded = settlement.encode_base64().ok_or_else(|| {
        PaygateError::SettlementAborted("cannot encode error settlement".to_owned())
    })?;
    HeaderValue::from_bytes(encoded.as_ref())
        .map_err(|e| PaygateError::SettlementAborted(e.to_string()))
}

/// Builds the HTTP response for an after-verify `SkipHandler` directive.
///
/// Matches foundation Go Gin: default `Content-Type: application/json`, body
/// from `directive.body` (`null` when unset), plus `Payment-Response`.
fn skip_handler_response(
    directive: &r402_core::SkipHandlerDirective,
    settlement: &wire::SettleResponse,
) -> Result<Response, PaygateError> {
    let content_type = directive
        .content_type
        .as_deref()
        .unwrap_or("application/json");
    let body_bytes = directive.body.as_ref().map_or_else(
        || b"null".to_vec(),
        |value| serde_json::to_vec(value).unwrap_or_else(|_| b"null".to_vec()),
    );
    let header_value = settlement_to_header(settlement)?;
    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(http::header::CONTENT_TYPE, content_type)
        .body(Body::from(body_bytes))
        .unwrap_or_else(|_| Response::new(Body::from(b"null".as_slice())));
    response
        .headers_mut()
        .insert("Payment-Response", header_value);
    ensure_expose_headers(response.headers_mut());
    Ok(response)
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

/// Maps a paygate verification failure to the HTTP status.
///
/// A `Permit2AllowanceRequired` inside `VerificationFailed` maps to 412;
/// everything else is 402.
fn inferred_status(err: &PaygateError) -> StatusCode {
    if let PaygateError::VerificationFailed(message) = err
        && message.contains("permit2_allowance_required")
    {
        return StatusCode::PRECONDITION_FAILED;
    }
    StatusCode::PAYMENT_REQUIRED
}

fn build_settle_request(
    payload: &PaymentPayload,
    requirements: &wire::PaymentRequirements,
) -> Result<wire::SettleRequest, String> {
    let verify: wire::TypedVerifyRequest<2, PaymentPayload, wire::PaymentRequirements> =
        wire::TypedVerifyRequest {
            x402_version: wire::V2,
            payment_payload: payload.clone(),
            payment_requirements: requirements.clone(),
        };
    let json = serde_json::to_value(&verify).map_err(|e| e.to_string())?;
    Ok(wire::SettleRequest::from(json))
}

#[cfg(test)]
mod tests {
    use std::future::Future;

    use super::*;

    #[test]
    fn adds_header_when_absent() {
        let mut headers = HeaderMap::new();
        ensure_expose_headers(&mut headers);
        assert_eq!(
            headers.get(ACCESS_CONTROL_EXPOSE_HEADERS).unwrap(),
            X402_EXPOSED_HEADERS,
        );
    }

    #[test]
    fn merges_existing_header() {
        let mut headers = HeaderMap::new();
        let _ = headers.insert(
            ACCESS_CONTROL_EXPOSE_HEADERS,
            HeaderValue::from_static("X-Foo"),
        );
        ensure_expose_headers(&mut headers);
        let value = headers.get(ACCESS_CONTROL_EXPOSE_HEADERS).unwrap();
        let value = value.to_str().unwrap();
        assert!(value.contains("X-Foo"));
        assert!(value.contains("Payment-Required"));
        assert!(value.contains("Payment-Response"));
    }

    #[test]
    fn expose_headers_idempotent() {
        let mut headers = HeaderMap::new();
        ensure_expose_headers(&mut headers);
        ensure_expose_headers(&mut headers);
        let value = headers.get(ACCESS_CONTROL_EXPOSE_HEADERS).unwrap();
        assert_eq!(value, X402_EXPOSED_HEADERS);
    }

    #[test]
    fn permit2_allowance_required_maps_to_412() {
        assert_eq!(
            reason_to_status(&ErrorReason::Permit2AllowanceRequired),
            StatusCode::PRECONDITION_FAILED,
        );
    }

    #[test]
    fn other_reasons_map_to_402() {
        for reason in [
            ErrorReason::InvalidPayload,
            ErrorReason::InvalidPaymentRequirements,
            ErrorReason::InvalidExactEvmPayloadSignature,
            ErrorReason::InsufficientFunds,
            ErrorReason::DuplicateSettlement,
            ErrorReason::InvalidExactSolanaPayloadMemoMismatch,
            ErrorReason::InvalidTransactionState,
            ErrorReason::UnexpectedSettleError,
            ErrorReason::Custom("some_unknown_code".into()),
        ] {
            assert_eq!(
                reason_to_status(&reason),
                StatusCode::PAYMENT_REQUIRED,
                "reason {reason:?} should map to 402"
            );
        }
    }

    struct StubFacilitator;

    impl Facilitator for StubFacilitator {
        fn verify(
            &self,
            _request: wire::VerifyRequest,
        ) -> impl Future<Output = Result<wire::VerifyResponse, r402_core::FacilitatorError>> + Send
        {
            std::future::ready(Ok(wire::VerifyResponse::valid("0xpayer")))
        }

        fn settle(
            &self,
            _request: wire::SettleRequest,
        ) -> impl Future<Output = Result<wire::SettleResponse, r402_core::FacilitatorError>> + Send
        {
            std::future::ready(Ok(wire::SettleResponse::Success {
                payer: "0xpayer".into(),
                transaction: "0xtx".into(),
                network: "eip155:1".into(),
                amount: Some("1".into()),
                extensions: wire::Extensions::new(),
            }))
        }

        fn supported(
            &self,
        ) -> impl Future<Output = Result<wire::SupportedResponse, r402_core::FacilitatorError>> + Send
        {
            std::future::ready(Ok(wire::SupportedResponse::default()))
        }
    }

    fn sample_requirements() -> wire::PaymentRequirements {
        wire::PaymentRequirements::new(
            "exact".into(),
            "eip155:1".parse().unwrap(),
            "1000000".into(),
            "0xpayee".into(),
            "0xasset".into(),
            60,
        )
    }

    fn payment_header(accepted: &wire::PaymentRequirements) -> HeaderMap {
        let payload = PaymentPayload::new(accepted.clone(), serde_json::json!({}));
        let encoded = Base64Bytes::encode(serde_json::to_vec(&payload).unwrap());
        let mut headers = HeaderMap::new();
        let _ = headers.insert(
            PAYMENT_HEADER,
            HeaderValue::from_bytes(encoded.as_ref()).unwrap(),
        );
        headers
    }

    #[tokio::test]
    async fn verify_only_rejects_max_timeout_mismatch() {
        let req = sample_requirements();
        let mut gate = Paygate::builder(StubFacilitator)
            .accept(wire::PriceTag::new(req.clone()))
            .build();
        gate.enrich_accepts().await;
        let mut accepted = req;
        accepted.max_timeout_seconds = 999;
        let err = gate
            .verify_only(&payment_header(&accepted))
            .await
            .unwrap_err();
        assert!(matches!(err, PaygateError::NoPaymentMatching));
    }

    #[tokio::test]
    async fn verify_only_rejects_static_extra_mismatch() {
        let req = sample_requirements().with_extra(serde_json::json!({
            "feePayer": "FeePayer111111111111111111111111111111111"
        }));
        let mut gate = Paygate::builder(StubFacilitator)
            .accept(wire::PriceTag::new(req.clone()))
            .build();
        gate.enrich_accepts().await;
        let accepted = req.with_extra(serde_json::json!({
            "feePayer": "OtherPayer1111111111111111111111111111111"
        }));
        let err = gate
            .verify_only(&payment_header(&accepted))
            .await
            .unwrap_err();
        assert!(matches!(err, PaygateError::NoPaymentMatching));
    }
}
