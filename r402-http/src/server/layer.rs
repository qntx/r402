//! Axum middleware for enforcing [x402](https://www.x402.org) payments on protected routes.
//!
//! This middleware validates incoming payment headers using a configured x402 facilitator,
//! and settles valid payments either before or after request execution (configurable).
//!
//! Returns a `402 Payment Required` response if the request lacks a valid payment.
//!
//! ## Settlement Timing
//!
//! By default, settlement occurs **after** the request is processed. You can change this behavior:
//!
//! - **[`X402Middleware::settle_before_execution`]** - Settle payment **before** request execution.
//! - **[`X402Middleware::settle_after_execution`]** - Settle payment **after** request execution (default).
//!   This allows processing the request before committing the payment on-chain.
//!
//! ## Configuration Notes
//!
//! - **[`X402Middleware::with_price_tag`]** sets the assets and amounts accepted for payment (static pricing).
//! - **[`X402Middleware::with_dynamic_price`]** sets a callback for dynamic pricing based on request context.
//! - **[`X402Middleware::with_base_url`]** sets the base URL for computing full resource URLs.
//!   If not set, defaults to `http://localhost/` (avoid in production).
//! - **[`X402LayerBuilder::with_description`]** is optional but helps the payer understand what is being paid for.
//! - **[`X402LayerBuilder::with_mime_type`]** sets the MIME type of the protected resource (default: `application/json`).
//! - **[`X402LayerBuilder::with_resource`]** explicitly sets the full URI of the protected resource.
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
use r402::facilitator::Facilitator;
use r402::proto::v2;
use tower::util::BoxCloneSyncService;
use tower::{Layer, Service};
use url::Url;

use super::facilitator::FacilitatorClient;
use super::paygate::{Paygate, ResourceInfoBuilder};
use super::pricing::{DynamicPriceTags, PriceTagSource, StaticPriceTags};

/// The main X402 middleware instance for enforcing x402 payments on routes.
///
/// Create a single instance per application and use it to build payment layers
/// for protected routes.
pub struct X402Middleware<F> {
    facilitator: F,
    base_url: Option<Url>,
    settle_before_execution: bool,
}

impl<F: Clone> Clone for X402Middleware<F> {
    fn clone(&self) -> Self {
        Self {
            facilitator: self.facilitator.clone(),
            base_url: self.base_url.clone(),
            settle_before_execution: self.settle_before_execution,
        }
    }
}

impl<F: std::fmt::Debug> std::fmt::Debug for X402Middleware<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X402Middleware")
            .field("facilitator", &self.facilitator)
            .field("base_url", &self.base_url)
            .field("settle_before_execution", &self.settle_before_execution)
            .finish()
    }
}

impl<F> X402Middleware<F> {
    /// Returns a reference to the underlying facilitator.
    pub const fn facilitator(&self) -> &F {
        &self.facilitator
    }
}

impl X402Middleware<Arc<FacilitatorClient>> {
    /// Creates a new middleware instance with a default facilitator URL.
    ///
    /// # Panics
    ///
    /// Panics if the facilitator URL is invalid.
    #[must_use]
    pub fn new(url: &str) -> Self {
        let facilitator = FacilitatorClient::try_from(url).expect("Invalid facilitator URL");
        Self {
            facilitator: Arc::new(facilitator),
            base_url: None,
            settle_before_execution: false,
        }
    }

    /// Creates a new middleware instance with a facilitator URL.
    ///
    /// # Errors
    ///
    /// Returns an error if the URL is invalid.
    pub fn try_new(url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let facilitator = FacilitatorClient::try_from(url)?;
        Ok(Self {
            facilitator: Arc::new(facilitator),
            base_url: None,
            settle_before_execution: false,
        })
    }

    /// Returns the configured facilitator URL.
    #[must_use]
    pub fn facilitator_url(&self) -> &Url {
        self.facilitator.base_url()
    }

    /// Sets the TTL for caching the facilitator's supported response.
    ///
    /// Default is 10 minutes. Use [`FacilitatorClient::without_supported_cache()`]
    /// to disable caching entirely.
    #[must_use]
    pub fn with_supported_cache_ttl(&self, ttl: Duration) -> Self {
        let inner = Arc::unwrap_or_clone(Arc::clone(&self.facilitator));
        let facilitator = Arc::new(inner.with_supported_cache_ttl(ttl));
        Self {
            facilitator,
            base_url: self.base_url.clone(),
            settle_before_execution: self.settle_before_execution,
        }
    }
}

impl TryFrom<&str> for X402Middleware<Arc<FacilitatorClient>> {
    type Error = Box<dyn std::error::Error>;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl TryFrom<String> for X402Middleware<Arc<FacilitatorClient>> {
    type Error = Box<dyn std::error::Error>;

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
    /// If [`X402LayerBuilder::with_resource`] is not called, this base URL is combined with
    /// each request's path/query to compute the resource. If not set, defaults to `http://localhost/`.
    ///
    /// In production, prefer calling `with_resource` or setting a precise `base_url`.
    #[must_use]
    pub fn with_base_url(&self, base_url: Url) -> Self {
        let mut this = self.clone();
        this.base_url = Some(base_url);
        this
    }

    /// Enables settlement prior to request execution.
    /// When disabled (default), settlement occurs after successful request execution.
    #[must_use]
    pub fn settle_before_execution(&self) -> Self {
        let mut this = self.clone();
        this.settle_before_execution = true;
        this
    }

    /// Disables settlement prior to request execution (default behavior).
    ///
    /// When disabled, settlement occurs after successful request execution.
    /// This is the default behavior and allows the application to process
    /// the request before committing the payment on-chain.
    #[must_use]
    pub fn settle_after_execution(&self) -> Self {
        let mut this = self.clone();
        this.settle_before_execution = false;
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
        price_tag: v2::PriceTag,
    ) -> X402LayerBuilder<StaticPriceTags, TFacilitator> {
        X402LayerBuilder {
            facilitator: self.facilitator.clone(),
            price_source: StaticPriceTags::new(vec![price_tag]),
            base_url: self.base_url.clone().map(Arc::new),
            resource: Arc::new(ResourceInfoBuilder::default()),
            settle_before_execution: self.settle_before_execution,
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
    ) -> X402LayerBuilder<DynamicPriceTags, TFacilitator>
    where
        F: Fn(&HeaderMap, &Uri, Option<&Url>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Vec<v2::PriceTag>> + Send + 'static,
    {
        X402LayerBuilder {
            facilitator: self.facilitator.clone(),
            price_source: DynamicPriceTags::new(callback),
            base_url: self.base_url.clone().map(Arc::new),
            resource: Arc::new(ResourceInfoBuilder::default()),
            settle_before_execution: self.settle_before_execution,
        }
    }
}

/// Builder for configuring the X402 middleware layer.
///
/// Generic over `TSource` which implements [`PriceTagSource`] to support
/// both static and dynamic pricing strategies.
#[derive(Clone)]
#[allow(missing_debug_implementations)] // generic types may not implement Debug
pub struct X402LayerBuilder<TSource, TFacilitator> {
    facilitator: TFacilitator,
    settle_before_execution: bool,
    base_url: Option<Arc<Url>>,
    price_source: TSource,
    resource: Arc<ResourceInfoBuilder>,
}

impl<TFacilitator> X402LayerBuilder<StaticPriceTags, TFacilitator> {
    /// Adds another payment option.
    ///
    /// Allows specifying multiple accepted payment methods (e.g., different networks).
    ///
    /// Note: This method is only available for static price tag sources.
    #[must_use]
    pub fn with_price_tag(mut self, price_tag: v2::PriceTag) -> Self {
        self.price_source = self.price_source.with_price_tag(price_tag);
        self
    }
}

#[allow(missing_debug_implementations)] // generic types may not implement Debug
impl<TSource, TFacilitator> X402LayerBuilder<TSource, TFacilitator> {
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
    #[allow(clippy::needless_pass_by_value)] // Url consumed via to_string()
    pub fn with_resource(mut self, resource: Url) -> Self {
        let mut new_resource = (*self.resource).clone();
        new_resource.url = Some(resource.to_string());
        self.resource = Arc::new(new_resource);
        self
    }
}

impl<S, TSource, TFacilitator> Layer<S> for X402LayerBuilder<TSource, TFacilitator>
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
            settle_before_execution: self.settle_before_execution,
            base_url: self.base_url.clone(),
            price_source: self.price_source.clone(),
            resource: Arc::clone(&self.resource),
            inner: BoxCloneSyncService::new(inner),
        }
    }
}

/// Axum service that enforces x402 payments on incoming requests.
///
/// Generic over `TSource` which implements [`PriceTagSource`] to support
/// both static and dynamic pricing strategies.
#[derive(Clone)]
#[allow(missing_debug_implementations)] // BoxCloneSyncService does not implement Debug
pub struct X402MiddlewareService<TSource, TFacilitator> {
    /// Payment facilitator (local or remote)
    facilitator: TFacilitator,
    /// Base URL for constructing resource URLs
    base_url: Option<Arc<Url>>,
    /// Whether to settle payment before executing the request (true) or after (false)
    settle_before_execution: bool,
    /// Price tag source - can be static or dynamic
    price_source: TSource,
    /// Resource information
    resource: Arc<ResourceInfoBuilder>,
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
    fn call(&mut self, req: Request) -> Self::Future {
        let price_source = self.price_source.clone();
        let facilitator = self.facilitator.clone();
        let base_url = self.base_url.clone();
        let resource_builder = Arc::clone(&self.resource);
        let settle_before_execution = self.settle_before_execution;
        let mut inner = self.inner.clone();

        Box::pin(async move {
            // Resolve price tags from the source
            let accepts = price_source
                .resolve(req.headers(), req.uri(), base_url.as_deref())
                .await;

            // If no price tags are configured, bypass payment enforcement
            if accepts.is_empty() {
                return inner.call(req).await;
            }

            let resource = resource_builder.as_resource_info(base_url.as_deref(), &req);

            let gate = {
                let mut gate = Paygate {
                    facilitator,
                    settle_before_execution,
                    accepts: Arc::new(accepts),
                    resource,
                };
                gate.enrich_accepts().await;
                gate
            };
            gate.handle_request(inner, req).await
        })
    }
}
