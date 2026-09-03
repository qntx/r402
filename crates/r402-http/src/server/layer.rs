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
use r402_server::{ResourceServer, SchemeNetworkServer};
use tower::util::BoxCloneSyncService;
use tower::{Layer, Service};
use url::Url;

use super::SettlementMode;
use super::fail::{GateError, abort_response};
use super::gate::Gate;
use super::hooks::{DynGateHooks, GateHooks, ProtectedRequestOutcome};
use super::pricing::{PriceTagSource, StaticPriceTags};

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
    pub(crate) hooks: Option<Arc<dyn DynGateHooks>>,
}

impl X402Layer<StaticPriceTags> {
    /// Adds another static payment option.
    #[must_use]
    pub fn with_price_tag(mut self, price_tag: PriceTag) -> Self {
        self.price_source = self.price_source.with_price_tag(price_tag);
        self
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

    /// Settlement scheduler. Only [`SettlementMode::Sequential`] is served.
    #[must_use]
    pub const fn with_settlement_mode(mut self, mode: SettlementMode) -> Self {
        self.settlement_mode = mode;
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
            hooks: self.hooks.clone(),
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
    hooks: Option<Arc<dyn DynGateHooks>>,
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
            self.hooks.clone(),
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
    hooks: Option<Arc<dyn DynGateHooks>>,
    mut inner: BoxCloneSyncService<Request, Response, Infallible>,
    req: Request,
) -> Result<Response, Infallible> {
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
    if accepts.is_empty() {
        return inner.call(req).await;
    }

    let resource = resource_builder.resolve(base_url.as_ref(), &req);
    let mut gate = Gate::from_parts(server, accepts, resource, settlement_mode, hooks);
    if let Err(err) = gate.require_schemes().await {
        return Ok(gate.error_response(err));
    }
    if let Err(err) = gate.assert_mode_compatible(settlement_mode) {
        return Ok(gate.error_response(err));
    }
    if let Err(err) = gate.build_payment_required().await {
        return Ok(gate.error_response(err));
    }

    let result = match settlement_mode {
        SettlementMode::Sequential => gate.handle_request(inner, req).await,
        SettlementMode::Concurrent | SettlementMode::Background => {
            Err(GateError::UnsupportedSettlementMode {
                mode: settlement_mode,
            })
        }
    };
    Ok(result.unwrap_or_else(|err| gate.error_response(err)))
}
