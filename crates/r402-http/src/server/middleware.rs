//! Axum middleware for enforcing [x402](https://www.x402.org) payments on protected routes.
//!
//! This middleware validates incoming payment headers using a configured x402 facilitator,
//! verifies the payment, executes the request, and settles valid payments after successful
//! execution. If the handler returns an error (4xx/5xx), settlement is skipped.
//!
//! Returns a `402 Payment Required` response if the request lacks a valid payment.
//!
//! ## Settlement Modes
//!
//! - **[`SettlementMode::Sequential`]** (default): verify → execute → settle.
//!   Safer — settlement only runs after the handler succeeds.
//! - **[`SettlementMode::Concurrent`]**: verify → (settle ∥ execute) → await settle.
//!   Lower latency — overlaps settlement with handler execution.
//! - **[`SettlementMode::Background`]**: verify → spawn settle → execute → return.
//!   Fire-and-forget — ideal for streaming responses.
//!
//! ## Configuration Notes
//!
//! - **[`X402Middleware::with_price_tag`]** sets the assets and amounts accepted for payment (static pricing).
//! - **[`X402Middleware::with_dynamic_price`]** sets a callback for dynamic pricing based on request context.
//! - **[`X402Middleware::with_base_url`]** sets the base URL for computing full resource URLs.
//!   If not set, defaults to `http://localhost/` (avoid in production).
//! - **[`X402Layer::with_settlement_mode`]** selects sequential or concurrent settlement.
//! - **[`X402Layer::with_description`]** is optional but helps the payer understand what is being paid for.
//! - **[`X402Layer::with_mime_type`]** sets the MIME type of the protected resource (default: `application/json`).
//! - **[`X402Layer::with_resource`]** explicitly sets the full URI of the protected resource.
//!

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum_core::extract::Request;
use axum_core::response::Response;
use http::{HeaderMap, Uri};
use r402_core::facilitator::Facilitator;
use r402_core::wire;
use tower::util::BoxCloneSyncService;
use tower::{Layer, Service};
use url::Url;

use super::facilitator::{FacilitatorClient, FacilitatorClientError};
use super::hooks::{DynPaygateHooks, PaygateHooks, ProtectedRequestOutcome};
use super::paygate::{Paygate, ResourceTemplate};
use super::pricing::{DynamicPriceTags, PriceTagSource, StaticPriceTags};

/// Controls when on-chain settlement executes relative to the inner service.
///
/// # Variants
///
/// - **Sequential** (default): verify → execute → settle.  Settlement only
///   runs after the handler returns a successful response.  This is the
///   safest option — no settlement occurs on handler errors.
///
/// - **Concurrent**: verify → (settle ∥ execute) → await settle.  Settlement
///   is spawned immediately after verification and runs in parallel with the
///   handler, reducing total request latency by one facilitator RTT.
///   On handler error the settlement task is detached (fire-and-forget).
///
/// - **Background**: verify → spawn settle (fire-and-forget) → execute → return.
///   Settlement runs entirely in the background — the response is returned to
///   the client immediately after the handler completes, without waiting for
///   settlement.  Ideal for **streaming** responses (e.g. SSE / LLM token
///   streams) where the client should start receiving data as soon as possible.
///   **Trade-off:** the `Payment-Response` header is not attached since settlement
///   may still be in progress when the response is sent.
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

/// The main X402 middleware instance for enforcing x402 payments on routes.
///
/// Create a single instance per application and use it to build payment layers
/// for protected routes.
pub struct X402Middleware<F> {
    facilitator: F,
    base_url: Option<Url>,
}

impl<F: Clone> Clone for X402Middleware<F> {
    fn clone(&self) -> Self {
        Self {
            facilitator: self.facilitator.clone(),
            base_url: self.base_url.clone(),
        }
    }
}

impl<F: std::fmt::Debug> std::fmt::Debug for X402Middleware<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X402Middleware")
            .field("facilitator", &self.facilitator)
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl<F> X402Middleware<F> {
    /// Creates a middleware instance from any facilitator implementation.
    ///
    /// Use this when you already have a configured facilitator (e.g. one
    /// with custom timeouts, caching, or a non-default HTTP client).
    #[must_use]
    pub const fn from_facilitator(facilitator: F) -> Self {
        Self {
            facilitator,
            base_url: None,
        }
    }

    /// Returns a reference to the underlying facilitator.
    pub const fn facilitator(&self) -> &F {
        &self.facilitator
    }
}

impl X402Middleware<Arc<FacilitatorClient>> {
    /// Creates a new middleware instance with a facilitator URL.
    ///
    /// # Errors
    ///
    /// Returns [`FacilitatorClientError`] if the URL cannot be parsed or the
    /// derived `/verify`, `/settle`, and `/supported` endpoints cannot be
    /// constructed.
    pub fn try_new(url: &str) -> Result<Self, FacilitatorClientError> {
        let facilitator = FacilitatorClient::try_from(url)?;
        Ok(Self {
            facilitator: Arc::new(facilitator),
            base_url: None,
        })
    }

    /// Returns the configured facilitator URL.
    #[must_use]
    pub fn facilitator_url(&self) -> &Url {
        self.facilitator.base_url()
    }

    /// Enables TTL caching of the facilitator's `/supported` response.
    ///
    /// Caching is off by default.
    #[must_use]
    pub fn with_supported_cache_ttl(&self, ttl: Duration) -> Self {
        let inner = Arc::unwrap_or_clone(Arc::clone(&self.facilitator));
        let facilitator = Arc::new(inner.with_supported_cache_ttl(ttl));
        Self {
            facilitator,
            base_url: self.base_url.clone(),
        }
    }

    /// Sets a per-request timeout for all facilitator HTTP calls (verify, settle, supported).
    ///
    /// Without this, the underlying `reqwest::Client` uses no timeout by default,
    /// which can cause requests to hang indefinitely if the facilitator is slow
    /// or unreachable, eventually triggering OS-level TCP timeouts (typically 2–5 minutes).
    ///
    /// A reasonable production value is 30 seconds.
    #[must_use]
    pub fn with_facilitator_timeout(&self, timeout: Duration) -> Self {
        let inner = Arc::unwrap_or_clone(Arc::clone(&self.facilitator));
        let facilitator = Arc::new(inner.with_timeout(timeout));
        Self {
            facilitator,
            base_url: self.base_url.clone(),
        }
    }
}

impl TryFrom<&str> for X402Middleware<Arc<FacilitatorClient>> {
    type Error = FacilitatorClientError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl TryFrom<String> for X402Middleware<Arc<FacilitatorClient>> {
    type Error = FacilitatorClientError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_new(&value)
    }
}

impl<F> X402Middleware<F>
where
    F: Clone,
{
    /// Sets the base URL used to construct resource URLs dynamically.
    ///
    /// If [`X402Layer::with_resource`] is not called, this base URL is combined with
    /// each request's path/query to compute the resource. If not set, defaults to `http://localhost/`.
    ///
    /// In production, prefer calling `with_resource` or setting a precise `base_url`.
    #[must_use]
    pub fn with_base_url(&self, base_url: Url) -> Self {
        let mut this = self.clone();
        this.base_url = Some(base_url);
        this
    }
}

impl<TFacilitator> X402Middleware<TFacilitator>
where
    TFacilitator: Clone,
{
    /// Sets the price tag for the protected route.
    ///
    /// Creates a layer builder that can be further configured with additional
    /// price tags and resource information.
    #[must_use]
    pub fn with_price_tag(
        &self,
        price_tag: wire::PriceTag,
    ) -> X402Layer<StaticPriceTags, TFacilitator> {
        X402Layer {
            facilitator: self.facilitator.clone(),
            price_source: StaticPriceTags::new(vec![price_tag]),
            base_url: self.base_url.clone().map(Arc::new),
            resource: Arc::new(ResourceTemplate::default()),
            settlement_mode: SettlementMode::default(),
            hooks: None,
        }
    }

    /// Sets multiple price tags for the protected route.
    ///
    /// Convenience method for services that accept several payment options
    /// (e.g. multiple tokens / networks).  Returns an empty-bypass builder
    /// when the list is empty — the middleware will pass requests through
    /// without payment enforcement.
    #[must_use]
    pub fn with_price_tags(
        &self,
        price_tags: Vec<wire::PriceTag>,
    ) -> X402Layer<StaticPriceTags, TFacilitator> {
        X402Layer {
            facilitator: self.facilitator.clone(),
            price_source: StaticPriceTags::new(price_tags),
            base_url: self.base_url.clone().map(Arc::new),
            resource: Arc::new(ResourceTemplate::default()),
            settlement_mode: SettlementMode::default(),
            hooks: None,
        }
    }

    /// Sets a dynamic price source for the protected route.
    ///
    /// The `callback` receives request headers, URI, and base URL, and returns
    /// a vector of V2 price tags.
    #[must_use]
    pub fn with_dynamic_price<F, Fut>(
        &self,
        callback: F,
    ) -> X402Layer<DynamicPriceTags, TFacilitator>
    where
        F: Fn(&HeaderMap, &Uri, Option<&Url>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Vec<wire::PriceTag>> + Send + 'static,
    {
        X402Layer {
            facilitator: self.facilitator.clone(),
            price_source: DynamicPriceTags::new(callback),
            base_url: self.base_url.clone().map(Arc::new),
            resource: Arc::new(ResourceTemplate::default()),
            settlement_mode: SettlementMode::default(),
            hooks: None,
        }
    }
}

/// Builder for configuring the X402 middleware layer.
///
/// Generic over `TSource` which implements [`PriceTagSource`] to support
/// both static and dynamic pricing strategies.
#[derive(Clone)]
#[allow(
    missing_debug_implementations,
    reason = "generic types may not impl Debug"
)]
pub struct X402Layer<TSource, TFacilitator> {
    facilitator: TFacilitator,
    base_url: Option<Arc<Url>>,
    price_source: TSource,
    resource: Arc<ResourceTemplate>,
    settlement_mode: SettlementMode,
    hooks: Option<Arc<dyn DynPaygateHooks>>,
}

impl<TFacilitator> X402Layer<StaticPriceTags, TFacilitator> {
    /// Adds another payment option.
    ///
    /// Allows specifying multiple accepted payment methods (e.g., different networks).
    ///
    /// Note: This method is only available for static price tag sources.
    #[must_use]
    pub fn with_price_tag(mut self, price_tag: wire::PriceTag) -> Self {
        self.price_source = self.price_source.with_price_tag(price_tag);
        self
    }
}

#[allow(
    missing_debug_implementations,
    reason = "generic types may not impl Debug"
)]
impl<TSource, TFacilitator> X402Layer<TSource, TFacilitator> {
    /// Sets a description of what the payment grants access to.
    ///
    /// This is included in 402 responses to inform clients what they're paying for.
    #[must_use]
    pub fn with_description(mut self, description: String) -> Self {
        let mut new_resource = (*self.resource).clone();
        new_resource.description = description;
        self.resource = Arc::new(new_resource);
        self
    }

    /// Sets the MIME type of the protected resource.
    ///
    /// Defaults to `application/json` if not specified.
    #[must_use]
    pub fn with_mime_type(mut self, mime: String) -> Self {
        let mut new_resource = (*self.resource).clone();
        new_resource.mime_type = mime;
        self.resource = Arc::new(new_resource);
        self
    }

    /// Sets the full URL of the protected resource.
    ///
    /// When set, this URL is used directly instead of constructing it from the base URL
    /// and request URI. This is the preferred approach in production.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "Url consumed via to_string()"
    )]
    pub fn with_resource(mut self, resource: Url) -> Self {
        let mut new_resource = (*self.resource).clone();
        new_resource.url = Some(resource.to_string());
        self.resource = Arc::new(new_resource);
        self
    }

    /// Sets the settlement mode.
    ///
    /// - [`SettlementMode::Sequential`] (default): verify → execute → settle.
    /// - [`SettlementMode::Concurrent`]: verify → (settle ∥ execute) → await settle.
    /// - [`SettlementMode::Background`]: verify → spawn settle → execute → return.
    ///
    /// Concurrent mode reduces total latency by overlapping settlement with
    /// handler execution. Background mode is ideal for streaming responses
    /// where the client should receive data immediately (settlement errors
    /// are logged but do not propagate).
    #[must_use]
    pub const fn with_settlement_mode(mut self, mode: SettlementMode) -> Self {
        self.settlement_mode = mode;
        self
    }

    /// Attaches [`PaygateHooks`] for pre- and post-payment extensibility.
    ///
    /// Hooks fire at the HTTP layer (before and after the x402 payment check)
    /// and let integrators bypass payment for API-key holders, enforce
    /// IP allow-lists, or short-circuit with a custom response.
    ///
    /// For transport-agnostic verify/settle lifecycle (including
    /// `on_verified_payment_canceled`), register hooks on a
    /// [`r402_core::ResourceServer`] and build the paygate with
    /// [`super::paygate::Paygate::builder_from_server`], or use
    /// [`super::paygate::PaygateBuilder::with_resource_hook`] on a manual
    /// [`Paygate`].
    #[must_use]
    pub fn with_hooks<H>(mut self, hooks: H) -> Self
    where
        H: PaygateHooks + 'static,
    {
        self.hooks = Some(Arc::new(hooks));
        self
    }
}

impl<S, TSource, TFacilitator> Layer<S> for X402Layer<TSource, TFacilitator>
where
    S: Service<Request, Response = Response, Error = Infallible> + Clone + Send + Sync + 'static,
    S::Future: Send + 'static,
    TFacilitator: Facilitator + Clone,
    TSource: PriceTagSource,
{
    type Service = X402MiddlewareService<TSource, TFacilitator>;

    fn layer(&self, inner: S) -> Self::Service {
        X402MiddlewareService {
            facilitator: self.facilitator.clone(),
            base_url: self.base_url.clone(),
            price_source: self.price_source.clone(),
            resource: Arc::clone(&self.resource),
            settlement_mode: self.settlement_mode,
            hooks: self.hooks.clone(),
            inner: BoxCloneSyncService::new(inner),
        }
    }
}

/// Axum service that enforces x402 payments on incoming requests.
///
/// Generic over `TSource` which implements [`PriceTagSource`] to support
/// both static and dynamic pricing strategies.
#[derive(Clone)]
#[allow(
    missing_debug_implementations,
    reason = "BoxCloneSyncService does not impl Debug"
)]
pub struct X402MiddlewareService<TSource, TFacilitator> {
    /// Payment facilitator (local or remote)
    facilitator: TFacilitator,
    /// Base URL for constructing resource URLs
    base_url: Option<Arc<Url>>,
    /// Price tag source - can be static or dynamic
    price_source: TSource,
    /// Resource information
    resource: Arc<ResourceTemplate>,
    /// Settlement strategy (sequential, concurrent, or background)
    settlement_mode: SettlementMode,
    /// Optional paygate lifecycle hooks (Fix-8)
    hooks: Option<Arc<dyn DynPaygateHooks>>,
    /// The inner Axum service being wrapped
    inner: BoxCloneSyncService<Request, Response, Infallible>,
}

impl<TSource, TFacilitator> Service<Request> for X402MiddlewareService<TSource, TFacilitator>
where
    TSource: PriceTagSource,
    TFacilitator: Facilitator + Clone + Send + Sync + 'static,
{
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    /// Delegates readiness polling to the wrapped inner service.
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    /// Intercepts the request, injects payment enforcement logic, and forwards to the wrapped service.
    #[allow(
        clippy::excessive_nesting,
        reason = "async move + match inside Box::pin is the idiomatic Service::call shape"
    )]
    fn call(&mut self, req: Request) -> Self::Future {
        let price_source = self.price_source.clone();
        let facilitator = self.facilitator.clone();
        let base_url = self.base_url.clone();
        let resource_builder = Arc::clone(&self.resource);
        let settlement_mode = self.settlement_mode;
        let hooks = self.hooks.clone();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            // Fix-8: dispatch on_protected_request before the payment check.
            // Hooks may grant access, short-circuit with an error, or let
            // the standard flow continue.
            let mut req = req;
            if let Some(h) = hooks.as_ref() {
                match h.on_protected_request(&req).await {
                    ProtectedRequestOutcome::Continue => {}
                    ProtectedRequestOutcome::GrantAccess => return inner.call(req).await,
                    ProtectedRequestOutcome::Abort { status, body } => {
                        return Ok(build_abort_response(status, body));
                    }
                }
            }

            // Resolve price tags from the source
            let accepts = price_source
                .resolve(req.headers(), req.uri(), base_url.as_deref())
                .await;

            // If no price tags are configured, bypass payment enforcement
            if accepts.is_empty() {
                return inner.call(req).await;
            }

            let resource = resource_builder.resolve(base_url.as_deref(), &req);

            let mut gate_builder = Paygate::builder(facilitator)
                .accepts(accepts)
                .resource(resource);
            if let Some(h) = hooks.as_ref() {
                gate_builder = gate_builder.hooks_dyn(Arc::clone(h));
            }
            let mut gate = gate_builder.build();
            gate.enrich_accepts().await;

            // Fix-8: after the paygate verifies the payment, fire
            // on_payment_verified so hooks can stamp request extensions
            // (e.g. payer address) for downstream handlers.
            if let Some(h) = hooks.as_ref() {
                h.on_payment_verified(&mut req).await;
            }

            let result = match settlement_mode {
                SettlementMode::Sequential => gate.handle_request(inner, req).await,
                SettlementMode::Concurrent => gate.handle_request_concurrent(inner, req).await,
                SettlementMode::Background => gate.handle_request_background(inner, req).await,
            };
            Ok(result.unwrap_or_else(|err| gate.error_response(err)))
        })
    }
}

/// Constructs an abort response from a [`PaygateHooks`] short-circuit.
fn build_abort_response(status: http::StatusCode, body: Option<String>) -> Response {
    let mut response = Response::new(axum_core::body::Body::from(body.unwrap_or_default()));
    *response.status_mut() = status;
    if let Ok(ct) = http::HeaderValue::from_str("text/plain; charset=utf-8") {
        let _ = response
            .headers_mut()
            .insert(http::header::CONTENT_TYPE, ct);
    }
    super::paygate::ensure_expose_headers(response.headers_mut());
    response
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "test assertions on known-valid fixtures"
)]
mod tests {
    use super::*;

    #[test]
    fn try_new_stores_parsed_facilitator_url() {
        let input = "https://facilitator.example.com";
        let middleware = X402Middleware::try_new(input).expect("valid facilitator URL");
        let expected = Url::parse("https://facilitator.example.com/").expect("fixture URL");
        assert_eq!(
            middleware.facilitator_url(),
            &expected,
            "stored base URL must equal the parsed, slash-normalized input"
        );
        assert_eq!(
            middleware.facilitator_url().as_str(),
            "https://facilitator.example.com/"
        );

        let slashed = X402Middleware::try_new("https://facilitator.example.com/")
            .expect("valid facilitator URL with trailing slash");
        assert_eq!(slashed.facilitator_url(), middleware.facilitator_url());
    }

    #[test]
    fn try_new_rejects_invalid_url() {
        let err = X402Middleware::try_new("not a url");
        assert!(
            err.is_err(),
            "invalid facilitator URL must return Err, not panic"
        );
        match err {
            Err(FacilitatorClientError::UrlParse { context, .. }) => {
                assert_eq!(context, "Failed to parse base url");
            }
            other => panic!("expected UrlParse, got {other:?}"),
        }
    }

    #[test]
    fn try_from_str_matches_try_new() {
        let via_try_new =
            X402Middleware::try_new("https://facilitator.example.com").expect("try_new");
        let via_try_from =
            X402Middleware::try_from("https://facilitator.example.com").expect("TryFrom<&str>");
        assert_eq!(
            via_try_new.facilitator_url(),
            via_try_from.facilitator_url()
        );
    }
}
