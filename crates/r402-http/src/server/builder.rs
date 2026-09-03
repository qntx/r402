//! [`X402Middleware`] construction. `with_price_tag` requires `base_url`.

use std::future::Future;
use std::sync::Arc;

use http::{HeaderMap, Uri};
use r402_facilitator::{Facilitator, FacilitatorClient, FacilitatorClientError};
use r402_protocol::network::ChainIdPattern;
use r402_protocol::payment::PriceTag;
use r402_server::{ResourceServer, SchemeNetworkServer};
use url::Url;

use super::SettlementMode;
use super::layer::{ResourceTemplate, X402Layer};
use super::pricing::{DynamicPriceTags, StaticPriceTags};

/// Construction failure. Currently only missing `base_url`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// `with_price_tag` / `with_price_tags` / `with_dynamic_price` without [`X402Middleware::with_base_url`].
    ///
    /// `with_resource` does not waive this requirement.
    #[error("base_url is required; call with_base_url before with_price_tag")]
    MissingBaseUrl,
}

/// Application-level x402 middleware. Owns a [`ResourceServer`].
///
/// Not a [`tower::Layer`]. Paid routes are built with [`Self::with_price_tag`].
#[derive(Clone, Debug)]
pub struct X402Middleware {
    server: ResourceServer,
    base_url: Option<Url>,
    #[cfg(feature = "siwx")]
    siwx: Option<Arc<super::SiwxGate>>,
}

impl X402Middleware {
    /// Wraps `fac` in a new [`ResourceServer`].
    #[must_use]
    pub fn from_facilitator(fac: impl Facilitator + 'static) -> Self {
        Self::from_resource_server(ResourceServer::new(Arc::new(fac)))
    }

    /// Creates middleware that owns an existing [`ResourceServer`].
    #[must_use]
    pub const fn from_resource_server(server: ResourceServer) -> Self {
        Self {
            server,
            base_url: None,
            #[cfg(feature = "siwx")]
            siwx: None,
        }
    }

    /// Remote facilitator from a base URL.
    ///
    /// # Errors
    ///
    /// [`FacilitatorClientError`] when the URL cannot be parsed or endpoints built.
    pub fn try_new(url: &str) -> Result<Self, FacilitatorClientError> {
        Ok(Self::from_facilitator(FacilitatorClient::try_from(url)?))
    }

    /// Registers a scheme/network adapter.
    #[must_use]
    pub fn with_scheme(
        mut self,
        network: ChainIdPattern,
        scheme: impl SchemeNetworkServer + 'static,
    ) -> Self {
        self.server.register_scheme(network, scheme);
        self
    }

    /// Public origin used to build `ResourceInfo.url`. Required before `with_price_tag`.
    ///
    /// Never derived from `Host` and never defaulted to `http://localhost`.
    #[must_use]
    pub fn with_base_url(&self, base_url: Url) -> Self {
        let mut this = self.clone();
        this.base_url = Some(base_url);
        this
    }

    /// Owned resource server.
    #[must_use]
    pub const fn resource_server(&self) -> &ResourceServer {
        &self.server
    }

    /// Enables SIWX on layers built from this middleware.
    ///
    /// Access is granted only with a valid CAIP-122 proof and a paid-address hit.
    /// Empty price tags still bypass unless [`Self::with_auth_only`].
    #[cfg(feature = "siwx")]
    #[must_use]
    pub fn with_siwx(&self, gate: super::SiwxGate) -> Self {
        let mut this = self.clone();
        this.siwx = Some(Arc::new(gate));
        this
    }

    /// Enables SIWX and treats empty price tags as auth-only (no layer bypass).
    ///
    /// Grants access on valid signature even without a paid-address hit.
    #[cfg(feature = "siwx")]
    #[must_use]
    pub fn with_auth_only(&self, gate: super::SiwxGate) -> Self {
        self.with_siwx(gate.with_auth_only())
    }

    /// Static single-tag layer. Fails when [`Self::with_base_url`] was not called.
    ///
    /// # Errors
    ///
    /// [`BuildError::MissingBaseUrl`].
    pub fn with_price_tag(
        &self,
        price_tag: PriceTag,
    ) -> Result<X402Layer<StaticPriceTags>, BuildError> {
        self.with_price_tags(vec![price_tag])
    }

    /// Static multi-tag layer. Empty list is a layer bypass after construct.
    ///
    /// # Errors
    ///
    /// [`BuildError::MissingBaseUrl`]. `with_resource` does not waive this.
    pub fn with_price_tags(
        &self,
        price_tags: Vec<PriceTag>,
    ) -> Result<X402Layer<StaticPriceTags>, BuildError> {
        Ok(X402Layer {
            server: self.server.clone(),
            price_source: StaticPriceTags::new(price_tags),
            base_url: Arc::new(self.base_url.clone().ok_or(BuildError::MissingBaseUrl)?),
            resource: Arc::new(ResourceTemplate::default()),
            settlement_mode: SettlementMode::default(),
            settlement_tracker: None,
            hooks: None,
            #[cfg(feature = "siwx")]
            siwx: self.siwx.clone(),
        })
    }

    /// Dynamic price source.
    ///
    /// # Errors
    ///
    /// [`BuildError::MissingBaseUrl`].
    pub fn with_dynamic_price<F, Fut>(
        &self,
        callback: F,
    ) -> Result<X402Layer<DynamicPriceTags>, BuildError>
    where
        F: Fn(&HeaderMap, &Uri, &Url) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Vec<PriceTag>> + Send + 'static,
    {
        Ok(X402Layer {
            server: self.server.clone(),
            price_source: DynamicPriceTags::new(callback),
            base_url: Arc::new(self.base_url.clone().ok_or(BuildError::MissingBaseUrl)?),
            resource: Arc::new(ResourceTemplate::default()),
            settlement_mode: SettlementMode::default(),
            settlement_tracker: None,
            hooks: None,
            #[cfg(feature = "siwx")]
            siwx: self.siwx.clone(),
        })
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
