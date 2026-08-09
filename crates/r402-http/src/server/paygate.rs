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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum_core::body::Body;
use axum_core::extract::Request;
use axum_core::response::{IntoResponse, Response};
use http::{HeaderMap, HeaderValue, StatusCode};
use r402_core::facilitator::{DynFacilitator, Facilitator};
use r402_core::resource_server::{CancelReason, ResourceServer, ResourceServerHooks};
use r402_core::wire;
use r402_core::wire::Base64Bytes;
use serde_json::json;
use tokio::sync::Notify;
use tower::Service;
#[cfg(feature = "telemetry")]
use tracing::{Instrument, instrument};
use url::Url;

use super::hooks::DynPaygateHooks;

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
/// facilitator with [`HookedFacilitator`](r402_core::hooks::HookedFacilitator)
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
            PaygateError::Verification(ve) => {
                let (status, payment_required) = {
                    // Fix-5: derive HTTP status from the inner ErrorReason when
                    // known — Permit2 allowance failures map to 412, others to 402.
                    let status = inferred_status(&ve);
                    let payment_required = wire::PaymentRequired::new(self.resource.clone())
                        .with_error(ve.to_string())
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
                super::cors::ensure_expose_headers(response.headers_mut());
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
                super::cors::ensure_expose_headers(response.headers_mut());
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
                super::cors::ensure_expose_headers(response.headers_mut());
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
    /// Returns [`PaygateError::Verification`] if the payment header is missing,
    /// malformed, or rejected by the facilitator / hooks.
    #[cfg_attr(feature = "telemetry", instrument(name = "x402.verify_only", skip_all))]
    pub async fn verify_only(&self, headers: &HeaderMap) -> Result<VerifiedPayment, PaygateError> {
        let header_bytes = headers
            .get(PAYMENT_HEADER)
            .map(HeaderValue::as_bytes)
            .ok_or(VerificationError::PaymentHeaderMissing)?;

        let payload: PaymentPayload =
            decode_payment_payload(header_bytes).ok_or(VerificationError::InvalidPaymentHeader)?;

        let requirements = match_requirements(&payload, &self.accepts)?;
        let outcome = self
            .server
            .verify_payment(&payload, &requirements)
            .await
            .map_err(|e| VerificationError::VerificationFailed(format!("{e}")))?;

        if let wire::VerifyResponse::Invalid { reason, .. } = &outcome.response {
            return Err(VerificationError::VerificationFailed(reason.to_string()).into());
        }

        // Build settle request from the same payload/requirements pair.
        let settle_request = build_settle_request(&payload, &requirements).map_err(|e| {
            VerificationError::VerificationFailed(format!("settle request build failed: {e}"))
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
        super::cors::ensure_expose_headers(response.headers_mut());
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
        super::cors::ensure_expose_headers(res.headers_mut());
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

/// Shared in-flight counter for background settlement tasks.
///
/// Created by the operator at startup, attached to a [`Paygate`] via
/// [`PaygateBuilder::with_settlement_tracker`], and drained at shutdown
/// via [`Paygate::settlement_tracker`] + [`Self::wait_for_drain`]. The
/// implementation is
/// lock-free in the steady state: a single [`AtomicUsize`] for the
/// counter and a [`tokio::sync::Notify`] for the drain wake-up.
///
/// Cloning the tracker is cheap and shares state, so it can be passed to
/// multiple paygates serving the same shutdown channel (for example,
/// when one process hosts several routes behind different price tags).
#[derive(Clone, Debug)]
pub struct BackgroundSettlementTracker {
    inner: Arc<TrackerInner>,
}

#[derive(Debug)]
struct TrackerInner {
    in_flight: AtomicUsize,
    drained: Notify,
}

impl Default for BackgroundSettlementTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl BackgroundSettlementTracker {
    /// Constructs a tracker with zero in-flight tasks.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(TrackerInner {
                in_flight: AtomicUsize::new(0),
                drained: Notify::new(),
            }),
        }
    }

    /// Returns the current approximate number of in-flight settlement
    /// tasks. Useful for `/healthz` style readiness probes.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.inner.in_flight.load(Ordering::SeqCst)
    }

    /// Increments the in-flight counter and returns a guard that
    /// decrements it on drop. Internal: the paygate's
    /// `handle_request_background` is the only intended caller.
    fn start(&self) -> SettlementInFlightGuard {
        let _previous = self.inner.in_flight.fetch_add(1, Ordering::SeqCst);
        SettlementInFlightGuard {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Awaits the in-flight count to reach zero, bounded by `timeout`.
    /// Returns `Ok(())` once drained, or `Err(remaining)` after the
    /// deadline with the count of still-running tasks.
    ///
    /// # Errors
    ///
    /// Returns the count of in-flight tasks when the timeout elapses
    /// before the drain completes. Callers may then choose to abort the
    /// runtime, log, or extend the deadline.
    pub async fn wait_for_drain(&self, timeout: Duration) -> Result<(), usize> {
        if self.in_flight() == 0 {
            return Ok(());
        }
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.inner.drained.notified();
            tokio::pin!(notified);
            tokio::select! {
                () = &mut notified => {}
                () = tokio::time::sleep_until(deadline) => {
                    let remaining = self.in_flight();
                    return if remaining == 0 { Ok(()) } else { Err(remaining) };
                }
            }
            if self.in_flight() == 0 {
                return Ok(());
            }
        }
    }
}

/// Drop-guard returned by [`BackgroundSettlementTracker::start`].
///
/// On drop, decrements the in-flight counter and notifies any awaiter
/// blocked in [`BackgroundSettlementTracker::wait_for_drain`]. The guard
/// is `Send + Sync` so it can be carried across `await` points by the
/// background settlement supervisor.
#[derive(Debug)]
pub(crate) struct SettlementInFlightGuard {
    inner: Arc<TrackerInner>,
}

impl Drop for SettlementInFlightGuard {
    fn drop(&mut self) {
        let previous = self.inner.in_flight.fetch_sub(1, Ordering::SeqCst);
        if previous == 1 {
            // Last in-flight task drained — wake every waiting drainer.
            self.inner.drained.notify_waiters();
        }
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
    super::cors::ensure_expose_headers(response.headers_mut());
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

/// Matches the payment payload against accepted price tags and builds a
/// [`wire::VerifyRequest`].
/// Maps an internal [`VerificationError`] to the correct HTTP status.
///
/// This is where Fix-5 lives: a `Permit2AllowanceRequired` inside the
/// `VerificationFailed(..)` string hints the buyer needs an on-chain
/// approval first — HTTP 412 is the canonical "precondition failed"
/// status per the x402 v2 spec.
fn inferred_status(ve: &VerificationError) -> StatusCode {
    if let VerificationError::VerificationFailed(message) = ve
        && message.contains("permit2_allowance_required")
    {
        return StatusCode::PRECONDITION_FAILED;
    }
    StatusCode::PAYMENT_REQUIRED
}

fn match_requirements(
    payload: &PaymentPayload,
    accepts: &[wire::PriceTag],
) -> Result<wire::PaymentRequirements, VerificationError> {
    accepts
        .iter()
        .find(|pt| **pt == payload.accepted)
        .map(|pt| pt.requirements.clone())
        .ok_or(VerificationError::NoPaymentMatching)
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
    use super::*;

    #[tokio::test]
    async fn empty_tracker_drains_immediately() {
        let tracker = BackgroundSettlementTracker::new();
        assert_eq!(tracker.in_flight(), 0);
        // No tasks in flight — drain returns Ok instantly even with a
        // zero deadline because the early-exit short-circuits the loop.
        tracker.wait_for_drain(Duration::ZERO).await.unwrap();
    }

    #[tokio::test]
    async fn drain_waits_for_guard_drop() {
        let tracker = BackgroundSettlementTracker::new();
        let guard = tracker.start();
        assert_eq!(tracker.in_flight(), 1);

        // Drop the guard from another task after a short delay; the main
        // task should observe the notify and return Ok.
        let tracker_clone = tracker.clone();
        let drop_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            drop(guard);
            assert_eq!(tracker_clone.in_flight(), 0);
        });

        tracker
            .wait_for_drain(Duration::from_secs(1))
            .await
            .expect("drain should complete after the guard drops");
        drop_task.await.unwrap();
    }

    #[tokio::test]
    async fn drain_times_out_when_guards_outlive_deadline() {
        let tracker = BackgroundSettlementTracker::new();
        let _guard = tracker.start();

        let result = tracker.wait_for_drain(Duration::from_millis(20)).await;
        assert_eq!(result, Err(1), "deadline elapses with the guard alive");
    }

    #[tokio::test]
    async fn nested_guards_decrement_in_order() {
        let tracker = BackgroundSettlementTracker::new();
        let g1 = tracker.start();
        let g2 = tracker.start();
        let g3 = tracker.start();
        assert_eq!(tracker.in_flight(), 3);
        drop(g2);
        assert_eq!(tracker.in_flight(), 2);
        drop(g1);
        assert_eq!(tracker.in_flight(), 1);
        drop(g3);
        assert_eq!(tracker.in_flight(), 0);
    }
}
