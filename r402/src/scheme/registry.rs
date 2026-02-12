//! Scheme handler builder, blueprint trait, and handler registry.
//!
//! [`SchemeBuilder`] defines how to construct a [`Facilitator`] from a
//! chain provider.  [`SchemeBlueprint`] combines identity ([`SchemeId`]) with
//! building capability so the registry can create handlers in a single call.
//!
//! [`SchemeRegistry`] holds the active handler instances keyed by chain+scheme.

use std::collections::HashMap;
use std::fmt;
use std::fmt::{Debug, Display, Formatter};

use super::SchemeId;
use crate::chain::{ChainId, ChainProvider};
use crate::facilitator::{BoxFuture, Facilitator, FacilitatorError};
use crate::proto;

/// Trait for building facilitator instances from chain providers.
///
/// The type parameter `P` represents the chain provider type.
pub trait SchemeBuilder<P> {
    /// Creates a new facilitator for the given chain provider.
    ///
    /// # Errors
    ///
    /// Returns an error if the facilitator cannot be built from the provider.
    fn build(
        &self,
        provider: P,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn Facilitator>, Box<dyn std::error::Error>>;
}

/// Marker trait for types that are both identifiable and buildable.
///
/// This combines [`SchemeId`] and [`SchemeBuilder`] so that the
/// registry can identify *and* construct handlers in a single call.
pub trait SchemeBlueprint<P>: SchemeId + for<'a> SchemeBuilder<&'a P> {}
impl<T, P> SchemeBlueprint<P> for T where T: SchemeId + for<'a> SchemeBuilder<&'a P> {}

/// Unique identifier for a scheme handler instance.
///
/// Combines the chain ID and scheme name to uniquely identify a handler
/// that can process payments for a specific chain+scheme combination.
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct SchemeSlug {
    /// The chain this handler operates on.
    pub chain_id: ChainId,
    /// The scheme name (e.g., "exact").
    pub name: String,
}

impl SchemeSlug {
    /// Creates a new scheme handler slug.
    #[must_use]
    pub const fn new(chain_id: ChainId, name: String) -> Self {
        Self { chain_id, name }
    }

    /// Returns a wildcard version of this slug that matches any chain
    /// within the same namespace.
    ///
    /// For example, `eip155:8453:exact` becomes `eip155:*:exact`.
    #[must_use]
    pub fn as_wildcard(&self) -> Self {
        Self {
            chain_id: ChainId::new(self.chain_id.namespace(), "*"),
            name: self.name.clone(),
        }
    }

    /// Returns `true` if this slug uses a wildcard reference (`*`).
    #[must_use]
    pub fn is_wildcard(&self) -> bool {
        self.chain_id.reference() == "*"
    }
}

impl Display for SchemeSlug {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.chain_id.namespace(),
            self.chain_id.reference(),
            self.name
        )
    }
}

/// Registry of active scheme handlers.
///
/// Maps chain+scheme combinations to their handlers.
#[derive(Default)]
pub struct SchemeRegistry(HashMap<SchemeSlug, Box<dyn Facilitator>>);

impl Debug for SchemeRegistry {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let slugs: Vec<String> = self.0.keys().map(ToString::to_string).collect();
        f.debug_tuple("SchemeRegistry").field(&slugs).finish()
    }
}

impl SchemeRegistry {
    /// Creates an empty scheme registry.
    #[must_use]
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    /// Registers a handler for a given blueprint and chain provider.
    ///
    /// # Errors
    ///
    /// Returns an error if the handler cannot be built from the provider.
    pub fn register<P: ChainProvider>(
        &mut self,
        blueprint: &dyn SchemeBlueprint<P>,
        provider: &P,
        config: Option<serde_json::Value>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let chain_id = provider.chain_id();
        let handler = blueprint.build(provider, config)?;
        let slug = SchemeSlug::new(chain_id, blueprint.scheme().to_string());
        self.0.insert(slug, handler);
        Ok(())
    }

    /// Gets a handler by its slug.
    ///
    /// Performs a two-phase lookup:
    /// 1. Exact match on the full slug (namespace:reference:scheme)
    /// 2. Wildcard fallback on the namespace (namespace:*:scheme)
    ///
    /// This allows registering a single handler for an entire namespace
    /// (e.g., `eip155:*`) that serves all chains within it.
    #[must_use]
    pub fn by_slug(&self, slug: &SchemeSlug) -> Option<&dyn Facilitator> {
        self.0
            .get(slug)
            .or_else(|| {
                let wildcard = slug.as_wildcard();
                self.0.get(&wildcard)
            })
            .map(|h| &**h)
    }

    /// Registers a handler for an entire namespace (wildcard).
    ///
    /// The handler will match any chain within the blueprint's namespace
    /// when no exact chain match is found.
    ///
    /// # Errors
    ///
    /// Returns an error if the handler cannot be built from the provider.
    pub fn register_for_namespace<P: ChainProvider>(
        &mut self,
        blueprint: &dyn SchemeBlueprint<P>,
        provider: &P,
        config: Option<serde_json::Value>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let handler = blueprint.build(provider, config)?;
        let namespace = provider.chain_id().namespace().to_owned();
        let slug = SchemeSlug::new(ChainId::new(namespace, "*"), blueprint.scheme().to_string());
        self.0.insert(slug, handler);
        Ok(())
    }

    /// Returns an iterator over all registered handlers.
    pub fn values(&self) -> impl Iterator<Item = &dyn Facilitator> {
        self.0.values().map(|v| &**v)
    }

    /// Looks up a handler by slug, returning an `Aborted` error if not found.
    fn require_handler(
        &self,
        slug: Option<SchemeSlug>,
    ) -> Result<&dyn Facilitator, FacilitatorError> {
        slug.and_then(|s| self.by_slug(&s))
            .ok_or_else(|| FacilitatorError::Aborted {
                reason: "no_facilitator_for_network".into(),
                message: "no handler registered for this payment scheme".into(),
            })
    }
}

impl Facilitator for SchemeRegistry {
    fn verify(
        &self,
        request: proto::VerifyRequest,
    ) -> BoxFuture<'_, Result<proto::VerifyResponse, FacilitatorError>> {
        Box::pin(async move {
            let handler = self.require_handler(request.scheme_slug())?;
            handler.verify(request).await
        })
    }

    fn settle(
        &self,
        request: proto::SettleRequest,
    ) -> BoxFuture<'_, Result<proto::SettleResponse, FacilitatorError>> {
        Box::pin(async move {
            let handler = self.require_handler(request.scheme_slug())?;
            handler.settle(request).await
        })
    }

    fn supported(
        &self,
    ) -> BoxFuture<'_, Result<proto::SupportedResponse, FacilitatorError>> {
        Box::pin(async move {
            let mut kinds = Vec::new();
            let mut signers: HashMap<String, Vec<String>> = HashMap::new();
            for handler in self.values() {
                if let Ok(mut resp) = handler.supported().await {
                    kinds.append(&mut resp.kinds);
                    for (family, addrs) in resp.signers {
                        signers.entry(family).or_default().extend(addrs);
                    }
                }
            }
            for addrs in signers.values_mut() {
                addrs.sort_unstable();
                addrs.dedup();
            }
            Ok(proto::SupportedResponse {
                kinds,
                extensions: Vec::new(),
                signers,
            })
        })
    }
}
