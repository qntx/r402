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

use std::fmt::{self, Display, Formatter};
use std::sync::Arc;

use axum_core::body::Body;
use axum_core::extract::Request;
use axum_core::response::{IntoResponse, Response};
use compact_str::CompactString;
use http::header::ACCESS_CONTROL_EXPOSE_HEADERS;
use http::{HeaderMap, HeaderValue, StatusCode};
use r402_core::SettlementOverrides;
use r402_core::chain::ChainId;
use r402_core::error::ErrorReason;
use r402_core::facilitator::{DynFacilitator, Facilitator};
use r402_core::resource_server::{
    CancelReason, CompletedSettlement, PaymentFlowError, PaymentFlowName, PaymentFlowPhases,
    PaymentRequiredBuildContext, ResourceServer, ResourceServerHooks, SettlePhase,
    build_failure_path_settlement_response, resolve_payment_flow_phases,
};
use r402_core::wire;
use r402_core::wire::{Base64Bytes, Extensions, PaymentRequired};
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

/// Controls when on-chain settlement executes relative to the inner service.
///
/// This is a latency scheduler for authorization `settleAfterHandler`. It is
/// not a substitute for `extra.paymentFlow`. Concurrent and Background are
/// illegal with upfront and escrow.
///
/// # Variants
///
/// - **Sequential** (default): verify → execute → settle. Settlement only
///   runs after the handler returns a successful response.
///
/// - **Concurrent**: verify → (settle ∥ execute) → await settle. Settlement
///   is spawned immediately after verification and runs in parallel with the
///   handler. Authorization only.
///
/// - **Background**: verify → spawn settle (fire-and-forget) → execute → return.
///   Authorization only. The `Payment-Response` header is not attached.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SettlementMode {
    /// Settlement runs **after** the handler completes.
    #[default]
    Sequential,
    /// Settlement runs **concurrently** with the handler; response waits for settlement.
    Concurrent,
    /// Settlement is fire-and-forget; response is returned immediately.
    Background,
}

impl SettlementMode {
    /// Stable lowercase name for error bodies.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::Concurrent => "concurrent",
            Self::Background => "background",
        }
    }
}

impl Display for SettlementMode {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
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
    /// 402 body construction failed (hook policy / payment-flow extra).
    #[error("payment-required construction failed: {0}")]
    PaymentRequiredBuild(String),
    /// Concurrent/Background used with an upfront or escrow accept.
    #[error("incompatible settlement mode {mode} with payment flow {flow}")]
    IncompatibleSettlementMode {
        /// Configured HTTP settlement mode.
        mode: SettlementMode,
        /// Payment flow on the offending accept.
        flow: PaymentFlowName,
    },
    /// No [`r402_core::SchemeNetworkServer`] registered for this accept.
    #[error("missing scheme {scheme} on {network}")]
    MissingScheme {
        /// Wire scheme name.
        scheme: CompactString,
        /// Accept network.
        network: ChainId,
    },
    /// Escrow accept whose scheme does not implement `settle_on_cancel`.
    #[error("escrow scheme {scheme} is missing settle_on_cancel")]
    MissingSettleOnCancel {
        /// Wire scheme name.
        scheme: CompactString,
    },
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
            payment_required: None,
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
    /// Shared 402 body for unpaid responses and paid matching.
    pub(crate) payment_required: Option<PaymentRequired>,
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

    /// Requires a registered [`r402_core::SchemeNetworkServer`] for every accept.
    ///
    /// Escrow additionally requires `settle_on_cancel` to return requirements.
    ///
    /// # Errors
    ///
    /// [`PaygateError::MissingScheme`], [`PaygateError::MissingSettleOnCancel`],
    /// or [`PaygateError::PaymentRequiredBuild`] when flow resolution fails.
    pub async fn require_schemes(&self) -> Result<(), PaygateError> {
        for tag in self.accepts.iter() {
            let requirements = &tag.requirements;
            if self
                .server
                .registered_scheme(requirements.scheme.as_str(), &requirements.network)
                .is_none()
            {
                return Err(PaygateError::MissingScheme {
                    scheme: requirements.scheme.clone(),
                    network: requirements.network.clone(),
                });
            }
            let flow = self.payment_flow_of(requirements)?;
            if flow == PaymentFlowName::Escrow
                && !self.server.has_settle_on_cancel(requirements).await
            {
                return Err(PaygateError::MissingSettleOnCancel {
                    scheme: requirements.scheme.clone(),
                });
            }
        }
        Ok(())
    }

    /// Concurrent/Background are illegal if any resolved accept is upfront or escrow.
    ///
    /// Checked on **all** tags, not only the matched one.
    ///
    /// # Errors
    ///
    /// [`PaygateError::IncompatibleSettlementMode`] or flow-resolution errors.
    pub fn assert_mode_compatible(&self, mode: SettlementMode) -> Result<(), PaygateError> {
        if mode == SettlementMode::Sequential {
            return Ok(());
        }
        for tag in self.accepts.iter() {
            let flow = self.payment_flow_of(&tag.requirements)?;
            if matches!(flow, PaymentFlowName::Upfront | PaymentFlowName::Escrow) {
                return Err(PaygateError::IncompatibleSettlementMode { mode, flow });
            }
        }
        Ok(())
    }

    fn payment_flow_of(
        &self,
        requirements: &wire::PaymentRequirements,
    ) -> Result<PaymentFlowName, PaygateError> {
        self.server
            .get_payment_flow(requirements)
            .map_err(|err| flow_error(err, &requirements.network))
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

    /// 402 body produced by [`Self::build_payment_required`], if built.
    #[must_use]
    pub const fn payment_required(&self) -> Option<&PaymentRequired> {
        self.payment_required.as_ref()
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
                let Some(mut payment_required) = self.payment_required.clone() else {
                    return json_status_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        &json!({ "error": "payment-required response has not been built" }),
                    );
                };
                let status = inferred_status(&err);
                payment_required.error = Some(err.to_string().into());
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
            PaygateError::PaymentRequiredBuild(ref detail) => json_status_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &json!({ "error": detail }),
            ),
            PaygateError::IncompatibleSettlementMode { mode, flow } => json_status_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &json!({
                    "error": "incompatible settlement mode",
                    "mode": mode.as_str(),
                    "flow": flow.as_str(),
                }),
            ),
            PaygateError::MissingScheme {
                ref scheme,
                ref network,
            } => json_status_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &json!({
                    "error": "missing scheme",
                    "scheme": scheme,
                    "network": network.to_string(),
                }),
            ),
            PaygateError::MissingSettleOnCancel { ref scheme } => json_status_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &json!({
                    "error": "missing settle_on_cancel",
                    "scheme": scheme,
                }),
            ),
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
    /// Builds the single 402 body used by unpaid responses and paid matching.
    ///
    /// Fetches `/supported` then runs
    /// [`ResourceServer::create_payment_required_response`] (scheme enrich is
    /// the only extra channel).
    ///
    /// # Errors
    ///
    /// [`PaygateError::PaymentRequiredBuild`] when the pipeline fails closed.
    pub async fn build_payment_required(&mut self) -> Result<(), PaygateError> {
        let facilitator = self.facilitator();
        let supported = Facilitator::supported(&facilitator)
            .await
            .unwrap_or_default();
        let reqs: Vec<_> = self
            .accepts
            .iter()
            .map(|pt| pt.requirements.clone())
            .collect();
        let built = self
            .server
            .create_payment_required_response(
                reqs,
                PaymentRequiredBuildContext {
                    resource: self.resource.clone(),
                    error: None,
                    extensions: Extensions::new(),
                    supported,
                    payment_payload: None,
                },
            )
            .await
            .map_err(|err| PaygateError::PaymentRequiredBuild(err.to_string()))?;
        self.payment_required = Some(built);
        Ok(())
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

        let payment_required = self.payment_required.as_ref().ok_or_else(|| {
            PaygateError::PaymentRequiredBuild(
                "payment-required response has not been built".into(),
            )
        })?;
        let requirements = self
            .server
            .find_matching_requirements(&payment_required.accepts, &payload)
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
    /// verify → settle-before-handler (upfront/escrow) → execute
    ///        → settle-after-handler (authorization/escrow) → attach header
    /// ```
    ///
    /// Handler 4xx/5xx runs cancel-settle and attaches official
    /// failure-path `Payment-Response` (cancel receipt, else deposit echo).
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
        if let Some(directive) = verified.skip_handler.clone() {
            return skip_handler_settle(&verified, &directive).await;
        }

        let flow = self.payment_flow_of(verified.requirements())?;
        let phases = resolve_payment_flow_phases(flow);
        let before_handler = settle_before_handler_if_needed(&verified, flow, phases).await?;
        let cancel = cancellation_guard(&verified, before_handler.as_ref());

        let response = match call_inner(inner, req).await {
            Ok(r) => r,
            Err(err) => {
                let cancel_settlement = cancel
                    .cancel(
                        CancelReason::HandlerThrew,
                        Some("inner service error"),
                        None,
                    )
                    .await;
                let mut response = err.into_response();
                attach_failure_path_settlement(
                    &mut response,
                    cancel_settlement.as_ref(),
                    before_handler.as_ref(),
                    Some(&verified.payload),
                )?;
                return Ok(response);
            }
        };

        if response.status().is_client_error() || response.status().is_server_error() {
            let cancel_settlement = cancel
                .cancel(
                    CancelReason::HandlerFailed,
                    Some("handler returned error status"),
                    Some(response.status().as_u16()),
                )
                .await;
            let mut response = response.into_response();
            attach_failure_path_settlement(
                &mut response,
                cancel_settlement.as_ref(),
                before_handler.as_ref(),
                Some(&verified.payload),
            )?;
            return Ok(response);
        }

        let mut response = response.into_response();
        // Upto: Settlement-Overrides header and/or UptoActualAmount extension.
        let override_amount =
            super::upto::resolve_response_settlement_amount(&mut response, verified.requirements())
                .map_err(|e| PaygateError::SettlementAborted(e.to_string()))?;
        let overrides = override_amount.as_deref().map(SettlementOverrides::amount);

        let settlement = verified
            .process_settlement(
                SettlePhase::AfterHandler,
                overrides.as_ref(),
                before_handler.as_ref(),
            )
            .await?;
        attach_payment_response(&mut response, &settlement)?;
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
        if let Some(directive) = verified.skip_handler.clone() {
            return skip_handler_settle(&verified, &directive).await;
        }

        let flow = self.payment_flow_of(verified.requirements())?;
        require_authorization_mode(SettlementMode::Concurrent, flow)?;
        let cancel = cancellation_guard(&verified, None);

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
        attach_payment_response(&mut res, &settlement)?;
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
        if let Some(directive) = verified.skip_handler.clone() {
            return skip_handler_settle(&verified, &directive).await;
        }

        let flow = self.payment_flow_of(verified.requirements())?;
        require_authorization_mode(SettlementMode::Background, flow)?;
        let cancel = cancellation_guard(&verified, None);

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
        let overrides = actual_amount.map(SettlementOverrides::amount);
        self.process_settlement(SettlePhase::AfterHandler, overrides.as_ref(), None)
            .await
    }

    /// Official `processSettlement`: [`SettlePhase::AfterHandler`] no-ops when
    /// `!settleAfterHandler` (echo before-handler receipt, or empty success).
    async fn process_settlement(
        &self,
        phase: SettlePhase,
        overrides: Option<&SettlementOverrides>,
        before_handler: Option<&CompletedSettlement>,
    ) -> Result<wire::SettleResponse, PaygateError> {
        let flow = self
            .server
            .get_payment_flow(&self.requirements)
            .map_err(|err| flow_error(err, &self.requirements.network))?;
        let phases = resolve_payment_flow_phases(flow);

        if phase != SettlePhase::BeforeHandler && !phases.settle_after_handler {
            if let Some(before) = before_handler {
                return Ok(before.result.clone());
            }
            return Ok(wire::SettleResponse::Success {
                payer: CompactString::default(),
                transaction: CompactString::default(),
                network: self.requirements.network.to_string().into(),
                amount: None,
                extensions: Extensions::new(),
                extra: None,
            });
        }

        let settlement = self
            .server
            .settle_payment(&self.payload, &self.requirements, overrides, phase)
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

fn flow_error(err: PaymentFlowError, network: &ChainId) -> PaygateError {
    match err {
        PaymentFlowError::UnregisteredScheme { scheme, .. } => PaygateError::MissingScheme {
            scheme: scheme.into(),
            network: network.clone(),
        },
        other => PaygateError::PaymentRequiredBuild(other.to_string()),
    }
}

const fn require_authorization_mode(
    mode: SettlementMode,
    flow: PaymentFlowName,
) -> Result<(), PaygateError> {
    if matches!(flow, PaymentFlowName::Upfront | PaymentFlowName::Escrow) {
        return Err(PaygateError::IncompatibleSettlementMode { mode, flow });
    }
    Ok(())
}

fn cancellation_guard(
    verified: &VerifiedPayment,
    before_handler: Option<&CompletedSettlement>,
) -> r402_core::CancellationGuard {
    let guard = verified.cancellation_guard();
    match before_handler {
        Some(completed) => guard
            .with_settled_phases([SettlePhase::BeforeHandler])
            .with_before_handler(completed.clone()),
        None => guard,
    }
}

async fn settle_before_handler_if_needed(
    verified: &VerifiedPayment,
    flow: PaymentFlowName,
    phases: PaymentFlowPhases,
) -> Result<Option<CompletedSettlement>, PaygateError> {
    if !phases.settle_before_handler {
        return Ok(None);
    }
    let result = verified
        .process_settlement(SettlePhase::BeforeHandler, None, None)
        .await?;
    Ok(Some(CompletedSettlement::new(
        SettlePhase::BeforeHandler,
        flow,
        result,
        verified.requirements().clone(),
    )))
}

async fn skip_handler_settle(
    verified: &VerifiedPayment,
    directive: &r402_core::SkipHandlerDirective,
) -> Result<Response, PaygateError> {
    let settlement = verified
        .process_settlement(SettlePhase::AfterHandler, None, None)
        .await?;
    skip_handler_response(directive, &settlement)
}

fn attach_payment_response(
    response: &mut Response,
    settlement: &wire::SettleResponse,
) -> Result<(), PaygateError> {
    let header_value = settlement_to_header(settlement)?;
    response
        .headers_mut()
        .insert("Payment-Response", header_value);
    ensure_expose_headers(response.headers_mut());
    Ok(())
}

/// Official `CreateFailurePathSettlementHeaders`: cancel receipt, else deposit echo.
fn attach_failure_path_settlement(
    response: &mut Response,
    cancel_settlement: Option<&wire::SettleResponse>,
    before_handler: Option<&CompletedSettlement>,
    payment_payload: Option<&PaymentPayload>,
) -> Result<(), PaygateError> {
    let Some(receipt) =
        build_failure_path_settlement_response(cancel_settlement, before_handler, payment_payload)
    else {
        return Ok(());
    };
    let encoded = receipt.encode_base64_any().ok_or_else(|| {
        PaygateError::SettlementAborted("cannot encode failure-path settlement".to_owned())
    })?;
    let header_value = HeaderValue::from_bytes(encoded.as_ref())
        .map_err(|e| PaygateError::SettlementAborted(e.to_string()))?;
    response
        .headers_mut()
        .insert("Payment-Response", header_value);
    ensure_expose_headers(response.headers_mut());
    Ok(())
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
#[allow(
    clippy::expect_used,
    reason = "infallible HTTP construction; panic indicates a bug"
)]
fn json_status_response(status: StatusCode, body: &serde_json::Value) -> Response {
    let mut response = Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("failed to construct response");
    ensure_expose_headers(response.headers_mut());
    response
}

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
    use std::collections::HashMap;
    use std::future::Future;

    use r402_core::chain::ChainIdPattern;
    use r402_core::resource_server::{
        PaymentFlowConfig, SDK_DEFAULT_ASSET_TRANSFER_METHOD, SchemeNetworkServer,
        SchemePaymentRequiredContext,
    };

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

    struct MockFacilitator {
        supported: wire::SupportedResponse,
    }

    impl Facilitator for MockFacilitator {
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
                network: "solana:mainnet".into(),
                amount: Some("1".into()),
                extensions: Extensions::new(),
                extra: None,
            }))
        }

        fn supported(
            &self,
        ) -> impl Future<Output = Result<wire::SupportedResponse, r402_core::FacilitatorError>> + Send
        {
            std::future::ready(Ok(self.supported.clone()))
        }
    }

    struct FeePayerScheme {
        flows: HashMap<String, PaymentFlowConfig>,
    }

    impl SchemeNetworkServer for FeePayerScheme {
        fn scheme(&self) -> &'static str {
            "exact"
        }

        fn default_asset_transfer_method(&self) -> &str {
            SDK_DEFAULT_ASSET_TRANSFER_METHOD
        }

        fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
            &self.flows
        }

        fn enrich_payment_required_response<'a>(
            &'a self,
            ctx: &'a SchemePaymentRequiredContext<'a>,
        ) -> impl Future<Output = Option<Vec<wire::PaymentRequirements>>> + Send + 'a {
            let fee_payer = ctx.supported.kinds.iter().find_map(|kind| {
                kind.extra
                    .as_ref()
                    .and_then(|extra| extra.get("feePayer"))
                    .cloned()
            });
            let mut accepts = ctx.requirements.to_vec();
            let Some(fee_payer) = fee_payer else {
                return std::future::ready(None);
            };
            for req in &mut accepts {
                let mut extra = req
                    .extra
                    .take()
                    .and_then(|value| value.as_object().cloned())
                    .unwrap_or_default();
                extra.insert("feePayer".into(), fee_payer.clone());
                req.extra = Some(serde_json::Value::Object(extra));
            }
            std::future::ready(Some(accepts))
        }
    }

    fn solana_requirements() -> wire::PaymentRequirements {
        wire::PaymentRequirements::new(
            "exact".into(),
            "solana:mainnet".parse().unwrap(),
            "1000".into(),
            "PayTo111111111111111111111111111111111111111".into(),
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into(),
            60,
        )
    }

    fn payment_headers(accepted: &wire::PaymentRequirements) -> HeaderMap {
        let payload = PaymentPayload::new(accepted.clone(), json!({"sig": "0x"}));
        let json = serde_json::to_vec(&payload).unwrap();
        let encoded = Base64Bytes::encode(json);
        let mut headers = HeaderMap::new();
        headers.insert(
            PAYMENT_HEADER,
            HeaderValue::from_bytes(encoded.as_ref()).unwrap(),
        );
        headers
    }

    async fn gate_with_fee_payer_enrich() -> Paygate {
        let supported = wire::SupportedResponse::new().with_kinds(vec![
            wire::SupportedPaymentKind::new(2, "exact", "solana:mainnet").with_extra(json!({
                "feePayer": "FeePayer1111111111111111111111111111111111111",
            })),
        ]);
        let mut flows = HashMap::new();
        flows.insert(
            SDK_DEFAULT_ASSET_TRANSFER_METHOD.into(),
            PaymentFlowConfig::authorization_and_upfront(),
        );
        let mut server = ResourceServer::new(Arc::new(MockFacilitator {
            supported: supported.clone(),
        }));
        server.register_scheme(ChainIdPattern::wildcard("solana"), FeePayerScheme { flows });
        let mut gate = Paygate::builder_from_server(server)
            .accept(wire::PriceTag::new(solana_requirements()))
            .resource(wire::ResourceInfo::new("https://example.com/paid"))
            .build();
        gate.build_payment_required().await.unwrap();
        gate
    }

    #[tokio::test]
    async fn omit_fee_payer_after_enrich_is_no_payment_matching() {
        let gate = gate_with_fee_payer_enrich().await;
        let pr = gate.payment_required().expect("built");
        let extra = pr.accepts.first().and_then(|req| req.extra.as_ref());
        assert_eq!(
            extra
                .and_then(|value| value.get("feePayer"))
                .and_then(serde_json::Value::as_str),
            Some("FeePayer1111111111111111111111111111111111111"),
        );

        let unpaid = gate.error_response(PaygateError::PaymentHeaderMissing);
        assert_eq!(unpaid.status(), StatusCode::PAYMENT_REQUIRED);
        assert!(unpaid.headers().get("Payment-Required").is_some());

        let omitted = solana_requirements();
        let err = gate
            .verify_only(&payment_headers(&omitted))
            .await
            .expect_err("omitted feePayer must not match");
        assert!(matches!(err, PaygateError::NoPaymentMatching), "{err:?}");

        let mut echoed = omitted;
        echoed.extra = pr.accepts.first().and_then(|req| req.extra.clone());
        gate.verify_only(&payment_headers(&echoed))
            .await
            .expect("echoed extra matches the 402 accepts list");
    }

    use std::convert::Infallible;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context as TaskContext, Poll};

    use r402_core::resource_server::{
        AfterVerifyDecision, BeforeOpDecision, ResourceServerHooks, SkipHandlerDirective,
        VerifyResultContext,
    };
    use r402_core::{PaymentFlowName, PaymentHookContext};

    use super::super::tracker::BackgroundSettlementTracker;

    struct CountingFacilitator {
        verifies: AtomicUsize,
        settles: AtomicUsize,
    }

    impl CountingFacilitator {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                verifies: AtomicUsize::new(0),
                settles: AtomicUsize::new(0),
            })
        }
    }

    impl Facilitator for CountingFacilitator {
        fn verify(
            &self,
            _request: wire::VerifyRequest,
        ) -> impl Future<Output = Result<wire::VerifyResponse, r402_core::FacilitatorError>> + Send
        {
            self.verifies.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(wire::VerifyResponse::valid("0xpayer")))
        }

        fn settle(
            &self,
            _request: wire::SettleRequest,
        ) -> impl Future<Output = Result<wire::SettleResponse, r402_core::FacilitatorError>> + Send
        {
            self.settles.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(wire::SettleResponse::Success {
                payer: "0xpayer".into(),
                transaction: "0xtx".into(),
                network: "eip155:1".into(),
                amount: Some("1000".into()),
                extensions: Extensions::new(),
                extra: None,
            }))
        }

        fn supported(
            &self,
        ) -> impl Future<Output = Result<wire::SupportedResponse, r402_core::FacilitatorError>> + Send
        {
            std::future::ready(Ok(wire::SupportedResponse::new()))
        }
    }

    struct FlowScheme {
        flows: HashMap<String, PaymentFlowConfig>,
        cancel: Option<wire::PaymentRequirements>,
    }

    impl FlowScheme {
        fn with_config(
            config: PaymentFlowConfig,
            cancel: Option<wire::PaymentRequirements>,
        ) -> Self {
            let mut flows = HashMap::new();
            flows.insert(SDK_DEFAULT_ASSET_TRANSFER_METHOD.into(), config);
            Self { flows, cancel }
        }

        fn authorization() -> Self {
            Self::with_config(PaymentFlowConfig::authorization_only(), None)
        }

        fn auth_and_upfront() -> Self {
            Self::with_config(PaymentFlowConfig::authorization_and_upfront(), None)
        }

        fn escrow(cancel: Option<wire::PaymentRequirements>) -> Self {
            Self::with_config(
                PaymentFlowConfig::new(vec![PaymentFlowName::Escrow], PaymentFlowName::Escrow),
                cancel,
            )
        }
    }

    impl SchemeNetworkServer for FlowScheme {
        fn scheme(&self) -> &'static str {
            "exact"
        }

        fn default_asset_transfer_method(&self) -> &str {
            SDK_DEFAULT_ASSET_TRANSFER_METHOD
        }

        fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
            &self.flows
        }

        fn settle_on_cancel<'a>(
            &'a self,
            _: &'a r402_core::VerifiedPaymentCanceledContext,
        ) -> impl Future<Output = Option<wire::PaymentRequirements>> + Send + 'a {
            std::future::ready(self.cancel.clone())
        }
    }

    struct SkipAfterVerify;

    impl ResourceServerHooks for SkipAfterVerify {
        fn after_verify<'a>(
            &'a self,
            _: &VerifyResultContext,
        ) -> impl Future<Output = AfterVerifyDecision> + Send + 'a {
            std::future::ready(AfterVerifyDecision::SkipHandler {
                response: SkipHandlerDirective::empty(),
            })
        }
    }

    struct SkipVerifyThenHandler;

    impl ResourceServerHooks for SkipVerifyThenHandler {
        fn before_verify<'a>(
            &'a self,
            _: &'a PaymentHookContext,
        ) -> impl Future<Output = BeforeOpDecision<wire::VerifyResponse>> + Send + 'a {
            std::future::ready(BeforeOpDecision::Skip {
                result: wire::VerifyResponse::valid("0xlocal"),
            })
        }

        fn after_verify<'a>(
            &'a self,
            _: &VerifyResultContext,
        ) -> impl Future<Output = AfterVerifyDecision> + Send + 'a {
            std::future::ready(AfterVerifyDecision::SkipHandler {
                response: SkipHandlerDirective::empty(),
            })
        }
    }

    #[derive(Clone)]
    struct StatusService {
        status: StatusCode,
    }

    impl Service<Request> for StatusService {
        type Response = Response;
        type Error = Infallible;
        type Future = std::future::Ready<Result<Response, Infallible>>;

        fn poll_ready(&mut self, _: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _: Request) -> Self::Future {
            let mut response = Response::new(Body::from("ok"));
            *response.status_mut() = self.status;
            std::future::ready(Ok(response))
        }
    }

    fn eip155_requirements() -> wire::PaymentRequirements {
        wire::PaymentRequirements::new(
            "exact".into(),
            "eip155:1".parse().unwrap(),
            "1000".into(),
            "0xpay".into(),
            "0xasset".into(),
            60,
        )
    }

    fn with_payment_flow(
        mut requirements: wire::PaymentRequirements,
        flow: &str,
    ) -> wire::PaymentRequirements {
        requirements.extra = Some(json!({ "paymentFlow": flow }));
        requirements
    }

    fn paid_request(accepted: &wire::PaymentRequirements) -> Request {
        let mut req = Request::new(Body::empty());
        *req.headers_mut() = payment_headers(accepted);
        req
    }

    async fn built_gate(
        fac: Arc<CountingFacilitator>,
        scheme: FlowScheme,
        tags: Vec<wire::PriceTag>,
        hook: Option<impl ResourceServerHooks + 'static>,
    ) -> Paygate {
        let mut server = ResourceServer::new(fac);
        server.register_scheme(ChainIdPattern::wildcard("eip155"), scheme);
        if let Some(hook) = hook {
            server.add_hook(hook);
        }
        let mut gate = Paygate::builder_from_server(server)
            .accepts(tags)
            .resource(wire::ResourceInfo::new("https://example.com/paid"))
            .build();
        gate.build_payment_required().await.unwrap();
        gate
    }

    async fn auth_gate(fac: Arc<CountingFacilitator>) -> Paygate {
        built_gate(
            fac,
            FlowScheme::authorization(),
            vec![wire::PriceTag::new(eip155_requirements())],
            None::<SkipAfterVerify>,
        )
        .await
    }

    #[tokio::test]
    async fn sequential_authorization_verifies_and_settles_after_handler() {
        let fac = CountingFacilitator::new();
        let gate = auth_gate(Arc::clone(&fac)).await;
        let response = gate
            .handle_request(
                StatusService {
                    status: StatusCode::OK,
                },
                paid_request(&eip155_requirements()),
            )
            .await
            .expect("sequential authorization");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get("Payment-Response").is_some());
        assert_eq!(fac.verifies.load(Ordering::SeqCst), 1);
        assert_eq!(fac.settles.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn sequential_upfront_settles_before_handler_and_echoes() {
        let fac = CountingFacilitator::new();
        let requirements = with_payment_flow(eip155_requirements(), "upfront");
        let gate = built_gate(
            Arc::clone(&fac),
            FlowScheme::auth_and_upfront(),
            vec![wire::PriceTag::new(requirements.clone())],
            None::<SkipAfterVerify>,
        )
        .await;
        let response = gate
            .handle_request(
                StatusService {
                    status: StatusCode::OK,
                },
                paid_request(&requirements),
            )
            .await
            .expect("sequential upfront");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get("Payment-Response").is_some());
        assert_eq!(fac.verifies.load(Ordering::SeqCst), 0);
        assert_eq!(fac.settles.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn sequential_escrow_settles_before_and_after_on_success() {
        let fac = CountingFacilitator::new();
        let requirements = with_payment_flow(eip155_requirements(), "escrow");
        let gate = built_gate(
            Arc::clone(&fac),
            FlowScheme::escrow(Some(requirements.clone())),
            vec![wire::PriceTag::new(requirements.clone())],
            None::<SkipAfterVerify>,
        )
        .await;
        let response = gate
            .handle_request(
                StatusService {
                    status: StatusCode::OK,
                },
                paid_request(&requirements),
            )
            .await
            .expect("sequential escrow");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(fac.verifies.load(Ordering::SeqCst), 0);
        assert_eq!(fac.settles.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn sequential_escrow_handler_4xx_runs_cancel_settle() {
        let fac = CountingFacilitator::new();
        let requirements = with_payment_flow(eip155_requirements(), "escrow");
        let gate = built_gate(
            Arc::clone(&fac),
            FlowScheme::escrow(Some(requirements.clone())),
            vec![wire::PriceTag::new(requirements.clone())],
            None::<SkipAfterVerify>,
        )
        .await;
        let response = gate
            .handle_request(
                StatusService {
                    status: StatusCode::BAD_REQUEST,
                },
                paid_request(&requirements),
            )
            .await
            .expect("handler 4xx is returned");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(response.headers().get("Payment-Response").is_some());
        assert_eq!(fac.verifies.load(Ordering::SeqCst), 0);
        // before-handler deposit + cancel settle
        assert_eq!(fac.settles.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn skip_handler_authorization_settles_after_handler() {
        let fac = CountingFacilitator::new();
        let mut server = ResourceServer::new(Arc::clone(&fac));
        server.register_scheme(
            ChainIdPattern::wildcard("eip155"),
            FlowScheme::authorization(),
        );
        server.add_hook(SkipAfterVerify);
        let mut gate = Paygate::builder_from_server(server)
            .accept(wire::PriceTag::new(eip155_requirements()))
            .resource(wire::ResourceInfo::new("https://example.com/paid"))
            .build();
        gate.build_payment_required().await.unwrap();
        let response = gate
            .handle_request(
                StatusService {
                    status: StatusCode::OK,
                },
                paid_request(&eip155_requirements()),
            )
            .await
            .expect("skip handler");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(fac.verifies.load(Ordering::SeqCst), 1);
        assert_eq!(fac.settles.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn skip_handler_upfront_does_not_settle() {
        let fac = CountingFacilitator::new();
        let requirements = with_payment_flow(eip155_requirements(), "upfront");
        let mut server = ResourceServer::new(Arc::clone(&fac));
        server.register_scheme(
            ChainIdPattern::wildcard("eip155"),
            FlowScheme::auth_and_upfront(),
        );
        server.add_hook(SkipVerifyThenHandler);
        let mut gate = Paygate::builder_from_server(server)
            .accept(wire::PriceTag::new(requirements.clone()))
            .resource(wire::ResourceInfo::new("https://example.com/paid"))
            .build();
        gate.build_payment_required().await.unwrap();
        let response = gate
            .handle_request(
                StatusService {
                    status: StatusCode::OK,
                },
                paid_request(&requirements),
            )
            .await
            .expect("skip handler upfront");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(fac.verifies.load(Ordering::SeqCst), 0);
        assert_eq!(fac.settles.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn concurrent_authorization_settles_in_parallel() {
        let fac = CountingFacilitator::new();
        let gate = auth_gate(Arc::clone(&fac)).await;
        let response = gate
            .handle_request_concurrent(
                StatusService {
                    status: StatusCode::OK,
                },
                paid_request(&eip155_requirements()),
            )
            .await
            .expect("concurrent authorization");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get("Payment-Response").is_some());
        assert_eq!(fac.verifies.load(Ordering::SeqCst), 1);
        assert_eq!(fac.settles.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn background_authorization_spawns_settle() {
        let fac = CountingFacilitator::new();
        let tracker = BackgroundSettlementTracker::new();
        let mut server = ResourceServer::new(Arc::clone(&fac));
        server.register_scheme(
            ChainIdPattern::wildcard("eip155"),
            FlowScheme::authorization(),
        );
        let mut gate = Paygate::builder_from_server(server)
            .accept(wire::PriceTag::new(eip155_requirements()))
            .resource(wire::ResourceInfo::new("https://example.com/paid"))
            .with_settlement_tracker(tracker.clone())
            .build();
        gate.build_payment_required().await.unwrap();
        let response = gate
            .handle_request_background(
                StatusService {
                    status: StatusCode::OK,
                },
                paid_request(&eip155_requirements()),
            )
            .await
            .expect("background authorization");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get("Payment-Response").is_none());
        tracker
            .wait_for_drain(std::time::Duration::from_secs(1))
            .await
            .expect("background settle drained");
        assert_eq!(fac.verifies.load(Ordering::SeqCst), 1);
        assert_eq!(fac.settles.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn require_schemes_errors_when_unregistered() {
        let gate = Paygate::builder(CountingFacilitator::new())
            .accept(wire::PriceTag::new(eip155_requirements()))
            .build();
        let err = gate.require_schemes().await.expect_err("missing scheme");
        assert!(matches!(err, PaygateError::MissingScheme { .. }), "{err:?}");
        assert_eq!(
            gate.error_response(err).status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn require_schemes_escrow_without_cancel_errors() {
        let fac = CountingFacilitator::new();
        let mut server = ResourceServer::new(fac);
        server.register_scheme(ChainIdPattern::wildcard("eip155"), FlowScheme::escrow(None));
        let gate = Paygate::builder_from_server(server)
            .accept(wire::PriceTag::new(eip155_requirements()))
            .build();
        let err = gate
            .require_schemes()
            .await
            .expect_err("missing settle_on_cancel");
        assert!(
            matches!(err, PaygateError::MissingSettleOnCancel { .. }),
            "{err:?}"
        );
        assert_eq!(
            gate.error_response(err).status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn concurrent_upfront_is_incompatible() {
        let fac = CountingFacilitator::new();
        let requirements = with_payment_flow(eip155_requirements(), "upfront");
        let mut server = ResourceServer::new(fac);
        server.register_scheme(
            ChainIdPattern::wildcard("eip155"),
            FlowScheme::auth_and_upfront(),
        );
        let gate = Paygate::builder_from_server(server)
            .accept(wire::PriceTag::new(requirements))
            .build();
        gate.require_schemes().await.unwrap();
        let err = gate
            .assert_mode_compatible(SettlementMode::Concurrent)
            .expect_err("upfront + concurrent");
        assert!(
            matches!(
                err,
                PaygateError::IncompatibleSettlementMode {
                    mode: SettlementMode::Concurrent,
                    flow: PaymentFlowName::Upfront,
                }
            ),
            "{err:?}"
        );
        assert_eq!(
            gate.error_response(err).status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn concurrent_mixed_authorization_and_escrow_is_incompatible() {
        let fac = CountingFacilitator::new();
        let auth = eip155_requirements();
        let escrow = with_payment_flow(eip155_requirements(), "escrow");
        let mut flows = HashMap::new();
        flows.insert(
            SDK_DEFAULT_ASSET_TRANSFER_METHOD.into(),
            PaymentFlowConfig::new(
                vec![PaymentFlowName::Authorization, PaymentFlowName::Escrow],
                PaymentFlowName::Authorization,
            ),
        );
        let mut server = ResourceServer::new(fac);
        server.register_scheme(
            ChainIdPattern::wildcard("eip155"),
            FlowScheme {
                flows,
                cancel: Some(escrow.clone()),
            },
        );
        let gate = Paygate::builder_from_server(server)
            .accept(wire::PriceTag::new(auth))
            .accept(wire::PriceTag::new(escrow))
            .build();
        gate.require_schemes().await.unwrap();
        let err = gate
            .assert_mode_compatible(SettlementMode::Background)
            .expect_err("mixed escrow under background");
        assert!(
            matches!(
                err,
                PaygateError::IncompatibleSettlementMode {
                    mode: SettlementMode::Background,
                    flow: PaymentFlowName::Escrow,
                }
            ),
            "{err:?}"
        );
    }
}
