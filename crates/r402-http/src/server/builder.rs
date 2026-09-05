//! [`X402Middleware`] construction. `with_price_tag` requires `base_url`.

use std::future::Future;
use std::sync::Arc;

use compact_str::CompactString;
use http::{HeaderMap, Uri};
use r402_facilitator::{Facilitator, FacilitatorClient, FacilitatorClientError};
use r402_protocol::extension::Extension;
use r402_protocol::network::{ChainId, ChainIdPattern};
use r402_protocol::payment::PriceTag;
use r402_server::{
    PaymentFlowError, PaymentFlowName, ResourceServer, SchemeNetworkServer, schedule,
};
use url::Url;

use super::SettlementMode;
use super::layer::{ResourceTemplate, X402Layer};
use super::pricing::{DynamicPriceTags, StaticPriceTags};

/// Construction failure for static HTTP layers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// `with_price_tag` / `with_price_tags` / `with_dynamic_price` without [`X402Middleware::with_base_url`].
    ///
    /// `with_resource` does not waive this requirement.
    #[error("base_url is required; call with_base_url before with_price_tag")]
    MissingBaseUrl,
    /// No scheme adapter is registered for this accept.
    #[error("missing scheme {scheme} on {network}")]
    MissingScheme {
        /// Wire scheme name.
        scheme: CompactString,
        /// Accept network.
        network: ChainId,
    },
    /// ATM / payment-flow resolution failed for a static tag.
    #[error(transparent)]
    PaymentFlow(#[from] PaymentFlowError),
    /// Concurrent or Background used with a flow that settles before the handler.
    #[error(transparent)]
    Mode(#[from] r402_server::IncompatibleSettlementMode),
    /// Empty static tags without SIWX `auth_only` already set.
    #[error(
        "with_price_tags([]) requires with_auth_only first; empty static tags are not a layer bypass"
    )]
    EmptyPriceTags,
    /// Escrow accept whose scheme does not implement cancel settle.
    #[error("escrow scheme {scheme} is missing settle_on_cancel")]
    MissingSettleOnCancel {
        /// Wire scheme name.
        scheme: CompactString,
    },
}

/// Rejects empty static tags (unless `auth_only`) and illegal scheme/flow/mode.
pub(crate) fn validate_static_layer(
    server: &ResourceServer,
    tags: &[PriceTag],
    mode: SettlementMode,
    auth_only: bool,
) -> Result<(), BuildError> {
    if tags.is_empty() {
        return if auth_only {
            Ok(())
        } else {
            Err(BuildError::EmptyPriceTags)
        };
    }

    for tag in tags {
        let requirements = &tag.requirements;
        let Some(scheme) =
            server.registered_scheme(requirements.scheme.as_str(), &requirements.network)
        else {
            return Err(BuildError::MissingScheme {
                scheme: requirements.scheme.clone(),
                network: requirements.network.clone(),
            });
        };

        let flow = match server.get_payment_flow(requirements) {
            Ok(flow) => flow,
            Err(PaymentFlowError::UnregisteredScheme { .. }) => {
                return Err(BuildError::MissingScheme {
                    scheme: requirements.scheme.clone(),
                    network: requirements.network.clone(),
                });
            }
            Err(err) => return Err(BuildError::PaymentFlow(err)),
        };

        if flow == PaymentFlowName::Escrow && !scheme.settles_on_cancel() {
            return Err(BuildError::MissingSettleOnCancel {
                scheme: requirements.scheme.clone(),
            });
        }

        schedule(flow.phases(), mode)?;
    }
    Ok(())
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

    /// Registers a protocol extension advertised on 402 responses.
    #[must_use]
    pub fn with_extension(mut self, extension: impl Extension + 'static) -> Self {
        self.server = self.server.with_extension(extension);
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
    /// Empty static tags still require [`Self::with_auth_only`] before
    /// [`Self::with_price_tags`].
    #[cfg(feature = "siwx")]
    #[must_use]
    pub fn with_siwx(&self, gate: super::SiwxGate) -> Self {
        let mut this = self.clone();
        this.siwx = Some(Arc::new(gate));
        this
    }

    /// Enables SIWX and treats empty price tags as auth-only.
    ///
    /// Call before [`Self::with_price_tags`] with an empty list. Grants access
    /// on valid signature even without a paid-address hit.
    #[cfg(feature = "siwx")]
    #[must_use]
    pub fn with_auth_only(&self, gate: super::SiwxGate) -> Self {
        self.with_siwx(gate.with_auth_only())
    }

    fn auth_only(&self) -> bool {
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

    /// Static single-tag layer. Fails when [`Self::with_base_url`] was not called.
    ///
    /// # Errors
    ///
    /// [`BuildError::MissingBaseUrl`], then other [`BuildError`] variants from
    /// static tag validation. `with_resource` does not waive `base_url`.
    pub fn with_price_tag(
        &self,
        price_tag: PriceTag,
    ) -> Result<X402Layer<StaticPriceTags>, BuildError> {
        self.with_price_tags(vec![price_tag])
    }

    /// Static multi-tag layer.
    ///
    /// Empty list is [`BuildError::EmptyPriceTags`] unless [`Self::with_auth_only`]
    /// was already called.
    ///
    /// # Errors
    ///
    /// [`BuildError::MissingBaseUrl`] when `with_base_url` was not called, or
    /// other [`BuildError`] variants when static tags are invalid.
    /// `with_resource` does not waive `base_url`.
    pub fn with_price_tags(
        &self,
        price_tags: Vec<PriceTag>,
    ) -> Result<X402Layer<StaticPriceTags>, BuildError> {
        let base_url = Arc::new(self.base_url.clone().ok_or(BuildError::MissingBaseUrl)?);
        validate_static_layer(
            &self.server,
            &price_tags,
            SettlementMode::default(),
            self.auth_only(),
        )?;
        Ok(X402Layer {
            server: self.server.clone(),
            price_source: StaticPriceTags::new(price_tags),
            base_url,
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
