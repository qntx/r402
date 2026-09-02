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
//! Concurrent and Background are authorization-only. Any resolved accept with
//! `upfront` or `escrow` under those modes is HTTP 500.
//!
//! ## Configuration Notes
//!
//! - **[`X402Middleware::with_scheme`]** registers a [`SchemeNetworkServer`] on the
//!   layer-owned [`ResourceServer`]. Paid routes 500 `MissingScheme` until then.
//! - **[`X402Middleware::with_price_tag`]** sets the assets and amounts accepted for payment (static pricing).
//! - **[`X402Middleware::with_dynamic_price`]** sets a callback for dynamic pricing based on request context.
//! - **[`X402Middleware::with_base_url`]** sets the base URL for computing full resource URLs.
//!   If not set, defaults to `http://localhost/` (avoid in production).
//! - **[`X402Layer::with_settlement_mode`]** selects sequential, concurrent, or background settlement.
//! - **[`X402Layer::with_description`]** is optional but helps the payer understand what is being paid for.
//! - **[`X402Layer::with_mime_type`]** sets the MIME type of the protected resource (default: `application/json`).
//! - **[`X402Layer::with_resource`]** explicitly sets the full URI of the protected resource.
//!

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum_core::extract::Request;
use axum_core::response::Response;
use http::{HeaderMap, Uri};
use r402_core::chain::ChainIdPattern;
use r402_core::facilitator::Facilitator;
use r402_core::resource_server::{ResourceServer, SchemeNetworkServer};
use r402_core::wire;
use tower::util::BoxCloneSyncService;
use tower::{Layer, Service};
use url::Url;

use super::facilitator::{FacilitatorClient, FacilitatorClientError};
use super::hooks::{DynPaygateHooks, PaygateHooks, ProtectedRequestOutcome};
use super::paygate::{Paygate, ResourceTemplate, SettlementMode};
use super::pricing::{DynamicPriceTags, PriceTagSource, StaticPriceTags};

/// The main X402 middleware instance for enforcing x402 payments on routes.
///
/// Owns a [`ResourceServer`] (facilitator + registered schemes). Create one
/// instance per application and use it to build payment layers for protected
/// routes. Paid routes return HTTP 500 `MissingScheme` until [`Self::with_scheme`].
#[derive(Clone, Debug)]
pub struct X402Middleware {
    server: ResourceServer,
    base_url: Option<Url>,
}

impl X402Middleware {
    /// Wraps `fac` in a new [`ResourceServer`] and calls [`Self::from_resource_server`].
    ///
    /// Paid routes 500 `MissingScheme` until [`Self::with_scheme`].
    #[must_use]
    pub fn from_facilitator(fac: impl Facilitator + 'static) -> Self {
        Self::from_resource_server(ResourceServer::new(Arc::new(fac)))
    }

    /// Creates middleware that owns an existing [`ResourceServer`] (schemes included).
    #[must_use]
    pub const fn from_resource_server(server: ResourceServer) -> Self {
        Self {
            server,
            base_url: None,
        }
    }

    /// Registers a scheme/network adapter on the owned [`ResourceServer`].
    #[must_use]
    pub fn with_scheme(
        mut self,
        network: ChainIdPattern,
        scheme: impl SchemeNetworkServer + 'static,
    ) -> Self {
        self.server.register_scheme(network, scheme);
        self
    }

    /// Returns a reference to the owned resource server.
    #[must_use]
    pub const fn resource_server(&self) -> &ResourceServer {
        &self.server
    }

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

    /// Sets the price tag for the protected route.
    ///
    /// Creates a layer builder that can be further configured with additional
    /// price tags and resource information.
    #[must_use]
    pub fn with_price_tag(&self, price_tag: wire::PriceTag) -> X402Layer<StaticPriceTags> {
        X402Layer {
            server: self.server.clone(),
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
    pub fn with_price_tags(&self, price_tags: Vec<wire::PriceTag>) -> X402Layer<StaticPriceTags> {
        X402Layer {
            server: self.server.clone(),
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
    pub fn with_dynamic_price<F, Fut>(&self, callback: F) -> X402Layer<DynamicPriceTags>
    where
        F: Fn(&HeaderMap, &Uri, Option<&Url>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Vec<wire::PriceTag>> + Send + 'static,
    {
        X402Layer {
            server: self.server.clone(),
            price_source: DynamicPriceTags::new(callback),
            base_url: self.base_url.clone().map(Arc::new),
            resource: Arc::new(ResourceTemplate::default()),
            settlement_mode: SettlementMode::default(),
            hooks: None,
        }
    }
}

impl X402Middleware {
    /// Creates a new middleware instance with a facilitator URL.
    ///
    /// # Errors
    ///
    /// Returns [`FacilitatorClientError`] if the URL cannot be parsed or the
    /// derived `/verify`, `/settle`, and `/supported` endpoints cannot be
    /// constructed.
    pub fn try_new(url: &str) -> Result<Self, FacilitatorClientError> {
        Ok(Self::from_facilitator(FacilitatorClient::try_from(url)?))
    }
}

impl TryFrom<&str> for X402Middleware {
    type Error = FacilitatorClientError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl TryFrom<String> for X402Middleware {
    type Error = FacilitatorClientError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_new(&value)
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
pub struct X402Layer<TSource> {
    server: ResourceServer,
    base_url: Option<Arc<Url>>,
    price_source: TSource,
    resource: Arc<ResourceTemplate>,
    settlement_mode: SettlementMode,
    hooks: Option<Arc<dyn DynPaygateHooks>>,
}

impl X402Layer<StaticPriceTags> {
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
impl<TSource> X402Layer<TSource> {
    /// Registers a scheme/network adapter on the owned [`ResourceServer`].
    #[must_use]
    pub fn with_scheme(
        mut self,
        network: ChainIdPattern,
        scheme: impl SchemeNetworkServer + 'static,
    ) -> Self {
        self.server.register_scheme(network, scheme);
        self
    }

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
    /// Concurrent and Background are authorization-only. Upfront or escrow on
    /// any resolved accept yields HTTP 500.
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
    /// [`r402_core::ResourceServer`] and pass it to
    /// [`X402Middleware::from_resource_server`], or use
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

impl<S, TSource> Layer<S> for X402Layer<TSource>
where
    S: Service<Request, Response = Response, Error = Infallible> + Clone + Send + Sync + 'static,
    S::Future: Send + 'static,
    TSource: PriceTagSource,
{
    type Service = X402MiddlewareService<TSource>;

    fn layer(&self, inner: S) -> Self::Service {
        X402MiddlewareService {
            server: self.server.clone(),
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
pub struct X402MiddlewareService<TSource> {
    /// Resource server (facilitator + schemes) copied from the layer.
    server: ResourceServer,
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

impl<TSource> Service<Request> for X402MiddlewareService<TSource>
where
    TSource: PriceTagSource,
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
        let server = self.server.clone();
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

            let mut gate_builder = Paygate::builder_from_server(server)
                .accepts(accepts)
                .resource(resource);
            if let Some(h) = hooks.as_ref() {
                gate_builder = gate_builder.hooks_dyn(Arc::clone(h));
            }
            let mut gate = gate_builder.build();
            if let Err(err) = gate.require_schemes().await {
                return Ok(gate.error_response(err));
            }
            if let Err(err) = gate.assert_mode_compatible(settlement_mode) {
                return Ok(gate.error_response(err));
            }
            if let Err(err) = gate.build_payment_required().await {
                return Ok(gate.error_response(err));
            }

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
    fn try_new_accepts_valid_facilitator_url() {
        assert!(X402Middleware::try_new("https://facilitator.example.com").is_ok());
        assert!(X402Middleware::try_new("https://facilitator.example.com/").is_ok());
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
        assert!(X402Middleware::try_new("https://facilitator.example.com").is_ok());
        assert!(X402Middleware::try_from("https://facilitator.example.com").is_ok());
        assert!(X402Middleware::try_from("https://facilitator.example.com".to_owned()).is_ok());
    }

    use std::collections::HashMap;
    use std::future::Future;

    use http::StatusCode;
    use r402_core::FacilitatorError;
    use r402_core::resource_server::{
        PaymentFlowConfig, SDK_DEFAULT_ASSET_TRANSFER_METHOD, SchemeNetworkServer,
    };
    use r402_core::wire::{self, Extensions};

    struct OkFacilitator;

    impl Facilitator for OkFacilitator {
        fn verify(
            &self,
            _request: wire::VerifyRequest,
        ) -> impl Future<Output = Result<wire::VerifyResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(wire::VerifyResponse::valid("0xpayer")))
        }

        fn settle(
            &self,
            _request: wire::SettleRequest,
        ) -> impl Future<Output = Result<wire::SettleResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(wire::SettleResponse::Success {
                payer: "0xpayer".into(),
                transaction: "0xtx".into(),
                network: "eip155:1".into(),
                amount: Some("1".into()),
                extensions: Extensions::new(),
                extra: None,
            }))
        }

        fn supported(
            &self,
        ) -> impl Future<Output = Result<wire::SupportedResponse, FacilitatorError>> + Send
        {
            std::future::ready(Ok(wire::SupportedResponse::new()))
        }
    }

    struct AuthScheme {
        flows: HashMap<String, PaymentFlowConfig>,
    }

    impl AuthScheme {
        fn authorization() -> Self {
            let mut flows = HashMap::new();
            flows.insert(
                SDK_DEFAULT_ASSET_TRANSFER_METHOD.into(),
                PaymentFlowConfig::authorization_only(),
            );
            Self { flows }
        }

        fn auth_and_upfront() -> Self {
            let mut flows = HashMap::new();
            flows.insert(
                SDK_DEFAULT_ASSET_TRANSFER_METHOD.into(),
                PaymentFlowConfig::authorization_and_upfront(),
            );
            Self { flows }
        }
    }

    impl SchemeNetworkServer for AuthScheme {
        fn scheme(&self) -> &'static str {
            "exact"
        }

        fn default_asset_transfer_method(&self) -> &str {
            SDK_DEFAULT_ASSET_TRANSFER_METHOD
        }

        fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
            &self.flows
        }
    }

    #[derive(Clone)]
    struct OkInner;

    impl Service<Request> for OkInner {
        type Response = Response;
        type Error = Infallible;
        type Future = std::future::Ready<Result<Response, Infallible>>;

        fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _: Request) -> Self::Future {
            std::future::ready(Ok(Response::new(axum_core::body::Body::from("ok"))))
        }
    }

    fn eip155_tag() -> wire::PriceTag {
        wire::PriceTag::new(wire::PaymentRequirements::new(
            "exact".into(),
            "eip155:1".parse().unwrap(),
            "1000".into(),
            "0xpay".into(),
            "0xasset".into(),
            60,
        ))
    }

    fn upfront_tag() -> wire::PriceTag {
        let mut requirements = eip155_tag().requirements;
        requirements.extra = Some(serde_json::json!({ "paymentFlow": "upfront" }));
        wire::PriceTag::new(requirements)
    }

    #[tokio::test]
    async fn paid_route_without_scheme_is_missing_scheme_500() {
        let layer = X402Middleware::from_facilitator(OkFacilitator).with_price_tag(eip155_tag());
        let mut svc = layer.layer(OkInner);
        let response = svc
            .call(Request::new(axum_core::body::Body::empty()))
            .await
            .expect("infallible");
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn concurrent_upfront_is_incompatible_500() {
        let layer = X402Middleware::from_facilitator(OkFacilitator)
            .with_scheme(
                ChainIdPattern::wildcard("eip155"),
                AuthScheme::auth_and_upfront(),
            )
            .with_price_tag(upfront_tag())
            .with_settlement_mode(SettlementMode::Concurrent);
        let mut svc = layer.layer(OkInner);
        let response = svc
            .call(Request::new(axum_core::body::Body::empty()))
            .await
            .expect("infallible");
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn concurrent_mixed_authorization_and_upfront_is_500() {
        let layer = X402Middleware::from_facilitator(OkFacilitator)
            .with_scheme(
                ChainIdPattern::wildcard("eip155"),
                AuthScheme::auth_and_upfront(),
            )
            .with_price_tags(vec![eip155_tag(), upfront_tag()])
            .with_settlement_mode(SettlementMode::Background);
        let mut svc = layer.layer(OkInner);
        let response = svc
            .call(Request::new(axum_core::body::Body::empty()))
            .await
            .expect("infallible");
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn sequential_with_registered_scheme_challenges_unpaid() {
        let layer = X402Middleware::from_facilitator(OkFacilitator)
            .with_scheme(
                ChainIdPattern::wildcard("eip155"),
                AuthScheme::authorization(),
            )
            .with_price_tag(eip155_tag())
            .with_settlement_mode(SettlementMode::Sequential);
        let mut svc = layer.layer(OkInner);
        let response = svc
            .call(Request::new(axum_core::body::Body::empty()))
            .await
            .expect("infallible");
        assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
        assert!(response.headers().get("Payment-Required").is_some());
    }
}
