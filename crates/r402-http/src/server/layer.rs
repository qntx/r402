//! [`X402Layer`]: Tower layer after a successful `with_price_tag`.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum_core::extract::Request;
use axum_core::response::Response;
use r402_protocol::network::ChainIdPattern;
use r402_protocol::payment::{PriceTag, ResourceInfo};
use r402_server::{BackgroundSettlementTracker, ResourceServer, SchemeNetworkServer};
use tower::util::BoxCloneSyncService;
use tower::{Layer, Service};
use url::Url;

use super::SettlementMode;
use super::builder::{BuildError, validate_static_layer};
use super::fail::abort_response;
use super::gate::Gate;
use super::hooks::{DynGateHooks, GateHooks, ProtectedRequestOutcome};
use super::pricing::{DynamicPriceTags, PriceTagSource, StaticPriceTags};

/// Route-level resource metadata. `url` pins `ResourceInfo.url`; it does not waive `base_url`.
#[derive(Debug, Clone)]
pub(crate) struct ResourceTemplate {
    pub(crate) description: String,
    pub(crate) mime_type: String,
    pub(crate) url: Option<Url>,
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
    /// Builds `ResourceInfo` from the pinned URL or `base_url` + request path/query.
    ///
    /// Never reads `Host` and never falls back to `http://localhost`.
    pub(crate) fn resolve(&self, base_url: &Url, req: &Request) -> ResourceInfo {
        let url = self.url.as_ref().map_or_else(
            || {
                let mut url = base_url.clone();
                url.set_path(req.uri().path());
                url.set_query(req.uri().query());
                url.to_string()
            },
            ToString::to_string,
        );
        let mut info = ResourceInfo::new(url);
        if !self.description.is_empty() {
            info = info.with_description(self.description.clone());
        }
        if !self.mime_type.is_empty() {
            info = info.with_mime_type(self.mime_type.clone());
        }
        info
    }
}

/// Tower layer produced by [`super::X402Middleware::with_price_tag`].
#[derive(Clone)]
pub struct X402Layer<TSource> {
    pub(crate) server: ResourceServer,
    pub(crate) base_url: Arc<Url>,
    pub(crate) price_source: TSource,
    pub(crate) resource: Arc<ResourceTemplate>,
    pub(crate) settlement_mode: SettlementMode,
    pub(crate) settlement_tracker: Option<BackgroundSettlementTracker>,
    pub(crate) hooks: Option<Arc<dyn DynGateHooks>>,
    #[cfg(feature = "siwx")]
    pub(crate) siwx: Option<Arc<super::SiwxGate>>,
}

impl X402Layer<StaticPriceTags> {
    /// Adds another static payment option.
    ///
    /// # Errors
    ///
    /// [`BuildError`] when the appended tag (or the existing list with the
    /// current settlement mode) is invalid.
    pub fn with_price_tag(mut self, price_tag: PriceTag) -> Result<Self, BuildError> {
        self.price_source = self.price_source.with_price_tag(price_tag);
        self.validate_static()?;
        Ok(self)
    }

    /// After-handler settle scheduler for the `authorization` payment flow.
    ///
    /// Not a `paymentFlow`. Sequential (default) is spec `authorization`:
    /// settle after the handler, then attach `Payment-Response`. Concurrent
    /// overlaps that settle with the handler and still waits. Background does
    /// not wait and does not attach a receipt. Concurrent and Background are
    /// illegal with `upfront` / `escrow`.
    ///
    /// # Errors
    ///
    /// [`BuildError::Mode`] when `mode` is incompatible with a static tag's flow.
    pub fn with_settlement_mode(mut self, mode: SettlementMode) -> Result<Self, BuildError> {
        self.settlement_mode = mode;
        self.validate_static()?;
        Ok(self)
    }

    /// Enables SIWX access-grant and 402 challenges on this route.
    ///
    /// # Errors
    ///
    /// [`BuildError::EmptyPriceTags`] when this replacement is not auth-only
    /// and the static tag list is empty.
    #[cfg(feature = "siwx")]
    pub fn with_siwx(mut self, gate: super::SiwxGate) -> Result<Self, BuildError> {
        self.siwx = Some(Arc::new(gate));
        self.validate_static()?;
        Ok(self)
    }

    /// Enables SIWX with auth-only access-grant on this route.
    ///
    /// # Errors
    ///
    /// [`BuildError`] from static re-validation after the replacement.
    #[cfg(feature = "siwx")]
    pub fn with_auth_only(self, gate: super::SiwxGate) -> Result<Self, BuildError> {
        self.with_siwx(gate.with_auth_only())
    }

    fn validate_static(&self) -> Result<(), BuildError> {
        validate_static_layer(
            &self.server,
            self.price_source.tags(),
            self.settlement_mode,
            self.static_auth_only(),
        )
    }

    fn static_auth_only(&self) -> bool {
        #[cfg(feature = "siwx")]
        {
            self.siwx.as_ref().is_some_and(|g| g.is_auth_only())
        }
        #[cfg(not(feature = "siwx"))]
        {
            let _ = self;
            false
        }
    }
}

impl X402Layer<DynamicPriceTags> {
    /// After-handler settle scheduler for the `authorization` payment flow.
    ///
    /// Same semantics as [`X402Layer<StaticPriceTags>::with_settlement_mode`].
    /// Dynamic tags are resolved per request; incompatible `upfront` /
    /// `escrow` combinations fail at request time, not here.
    ///
    /// # Errors
    ///
    /// Always `Ok` after storing `mode`.
    #[allow(
        clippy::unnecessary_wraps,
        clippy::missing_const_for_fn,
        reason = "same Result signature as Static"
    )]
    pub fn with_settlement_mode(mut self, mode: SettlementMode) -> Result<Self, BuildError> {
        self.settlement_mode = mode;
        Ok(self)
    }

    /// Enables SIWX access-grant and 402 challenges on this route.
    #[cfg(feature = "siwx")]
    #[must_use]
    pub fn with_siwx(mut self, gate: super::SiwxGate) -> Self {
        self.siwx = Some(Arc::new(gate));
        self
    }

    /// Enables SIWX with auth-only access-grant on this route.
    #[cfg(feature = "siwx")]
    #[must_use]
    pub fn with_auth_only(self, gate: super::SiwxGate) -> Self {
        self.with_siwx(gate.with_auth_only())
    }
}

impl<TSource: std::fmt::Debug> std::fmt::Debug for X402Layer<TSource> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X402Layer")
            .field("base_url", &self.base_url)
            .field("price_source", &self.price_source)
            .field("settlement_mode", &self.settlement_mode)
            .finish_non_exhaustive()
    }
}

impl<TSource> X402Layer<TSource> {
    /// Registers a scheme/network adapter on the owned resource server.
    #[must_use]
    pub fn with_scheme(
        mut self,
        network: ChainIdPattern,
        scheme: impl SchemeNetworkServer + 'static,
    ) -> Self {
        self.server.register_scheme(network, scheme);
        self
    }

    /// Description copied onto 402 `resource.description`.
    #[must_use]
    pub fn with_description(mut self, description: String) -> Self {
        let mut new_resource = (*self.resource).clone();
        new_resource.description = description;
        self.resource = Arc::new(new_resource);
        self
    }

    /// MIME type copied onto 402 `resource.mimeType`.
    #[must_use]
    pub fn with_mime_type(mut self, mime: String) -> Self {
        let mut new_resource = (*self.resource).clone();
        new_resource.mime_type = mime;
        self.resource = Arc::new(new_resource);
        self
    }

    /// Pins `ResourceInfo.url` for this route. Does **not** waive `base_url`.
    #[must_use]
    pub fn with_resource(mut self, resource: Url) -> Self {
        let mut new_resource = (*self.resource).clone();
        new_resource.url = Some(resource);
        self.resource = Arc::new(new_resource);
        self
    }

    /// In-flight counter for [`SettlementMode::Background`] settle tasks.
    #[must_use]
    pub fn with_settlement_tracker(mut self, tracker: BackgroundSettlementTracker) -> Self {
        self.settlement_tracker = Some(tracker);
        self
    }

    /// HTTP-layer hooks (`GrantAccess` slot).
    #[must_use]
    pub fn with_hooks<H>(mut self, hooks: H) -> Self
    where
        H: GateHooks + 'static,
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
            base_url: Arc::clone(&self.base_url),
            price_source: self.price_source.clone(),
            resource: Arc::clone(&self.resource),
            settlement_mode: self.settlement_mode,
            settlement_tracker: self.settlement_tracker.clone(),
            hooks: self.hooks.clone(),
            #[cfg(feature = "siwx")]
            siwx: self.siwx.clone(),
            inner: BoxCloneSyncService::new(inner),
        }
    }
}

/// Service produced by [`X402Layer`].
#[derive(Clone)]
#[allow(
    missing_debug_implementations,
    reason = "BoxCloneSyncService does not impl Debug"
)]
pub struct X402MiddlewareService<TSource> {
    server: ResourceServer,
    base_url: Arc<Url>,
    price_source: TSource,
    resource: Arc<ResourceTemplate>,
    settlement_mode: SettlementMode,
    settlement_tracker: Option<BackgroundSettlementTracker>,
    hooks: Option<Arc<dyn DynGateHooks>>,
    #[cfg(feature = "siwx")]
    siwx: Option<Arc<super::SiwxGate>>,
    inner: BoxCloneSyncService<Request, Response, Infallible>,
}

impl<TSource> Service<Request> for X402MiddlewareService<TSource>
where
    TSource: PriceTagSource,
{
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        Box::pin(enforce(
            self.price_source.clone(),
            self.server.clone(),
            Arc::clone(&self.base_url),
            Arc::clone(&self.resource),
            self.settlement_mode,
            self.settlement_tracker.clone(),
            self.hooks.clone(),
            #[cfg(feature = "siwx")]
            self.siwx.clone(),
            self.inner.clone(),
            req,
        ))
    }
}

#[allow(
    clippy::too_many_arguments,
    clippy::excessive_nesting,
    reason = "cloned service fields; hook match must borrow Request only for the await"
)]
async fn enforce<TSource: PriceTagSource>(
    price_source: TSource,
    server: ResourceServer,
    base_url: Arc<Url>,
    resource_builder: Arc<ResourceTemplate>,
    settlement_mode: SettlementMode,
    settlement_tracker: Option<BackgroundSettlementTracker>,
    hooks: Option<Arc<dyn DynGateHooks>>,
    #[cfg(feature = "siwx")] siwx: Option<Arc<super::SiwxGate>>,
    mut inner: BoxCloneSyncService<Request, Response, Infallible>,
    req: Request,
) -> Result<Response, Infallible> {
    #[cfg(feature = "siwx")]
    if let Some(siwx_gate) = siwx.as_ref() {
        let header = req
            .headers()
            .get(crate::headers::SIGN_IN_WITH_X)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let path = req.uri().path().to_owned();
        if let Some(header) = header
            && siwx_gate.try_grant(&header, &path).await
        {
            return inner.call(req).await;
        }
    }

    if let Some(h) = hooks.as_ref() {
        match h.on_protected_request(&req).await {
            ProtectedRequestOutcome::Continue => {}
            ProtectedRequestOutcome::GrantAccess => return inner.call(req).await,
            ProtectedRequestOutcome::Abort { status, body } => {
                return Ok(abort_response(status, body));
            }
        }
    }

    let accepts = price_source
        .resolve(req.headers(), req.uri(), base_url.as_ref())
        .await;
    #[cfg(feature = "siwx")]
    let auth_only = siwx.as_ref().is_some_and(|g| g.is_auth_only());
    #[cfg(not(feature = "siwx"))]
    let auth_only = false;
    if accepts.is_empty() && !auth_only {
        return inner.call(req).await;
    }

    let resource = resource_builder.resolve(base_url.as_ref(), &req);
    let mut gate = Gate::from_parts(
        server,
        accepts,
        resource,
        settlement_mode,
        settlement_tracker,
        hooks,
        #[cfg(feature = "siwx")]
        siwx.clone(),
    );
    if let Err(err) = gate.require_schemes().await {
        return Ok(gate.error_response(err));
    }
    if let Err(err) = gate.assert_mode_compatible(settlement_mode) {
        return Ok(gate.error_response(err));
    }
    if let Err(err) = gate.build_payment_required().await {
        return Ok(gate.error_response(err));
    }
    #[cfg(feature = "siwx")]
    if let Some(siwx) = siwx.as_ref()
        && let Err(err) = gate.attach_siwx_challenge(siwx, req.uri().path())
    {
        return Ok(gate.error_response(err));
    }

    let result = match settlement_mode {
        SettlementMode::Sequential => gate.handle_request(inner, req).await,
        SettlementMode::Concurrent => {
            // Join future holds handler + settle state (~17 KiB).
            Box::pin(gate.handle_request_concurrent(inner, req)).await
        }
        SettlementMode::Background => gate.handle_request_background(inner, req).await,
    };
    Ok(result.unwrap_or_else(|err| gate.error_response(err)))
}
