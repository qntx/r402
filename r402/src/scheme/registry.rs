//! Scheme handler builder, blueprint trait, and handler registry.
//!
//! [`SchemeHandlerBuilder`] defines how to construct a [`Facilitator`] from a
//! chain provider.  [`SchemeBlueprint`] combines identity ([`SchemeId`]) with
//! building capability so the registry can create handlers in a single call.
//!
//! [`SchemeRegistry`] holds the active handler instances keyed by chain+scheme.

use crate::chain::{ChainId, ChainProviderOps};
use crate::facilitator::Facilitator;

use std::collections::HashMap;
use std::fmt;
use std::fmt::{Debug, Display, Formatter};

use super::SchemeId;

/// Trait for building facilitator instances from chain providers.
///
/// The type parameter `P` represents the chain provider type.
pub trait SchemeHandlerBuilder<P> {
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
/// This combines [`SchemeId`] and [`SchemeHandlerBuilder`] so that the
/// registry can identify *and* construct handlers in a single call.
pub trait SchemeBlueprint<P>: SchemeId + for<'a> SchemeHandlerBuilder<&'a P> {}
impl<T, P> SchemeBlueprint<P> for T where T: SchemeId + for<'a> SchemeHandlerBuilder<&'a P> {}

/// Unique identifier for a scheme handler instance.
///
/// Combines the chain ID, protocol version, and scheme name to uniquely
/// identify a handler that can process payments for a specific combination.
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct SchemeHandlerSlug {
    /// The chain this handler operates on.
    pub chain_id: ChainId,
    /// The x402 protocol version.
    pub x402_version: u8,
    /// The scheme name (e.g., "exact").
    pub name: String,
}

impl SchemeHandlerSlug {
    /// Creates a new scheme handler slug.
    #[must_use]
    pub const fn new(chain_id: ChainId, x402_version: u8, name: String) -> Self {
        Self {
            chain_id,
            x402_version,
            name,
        }
    }

    /// Returns a wildcard version of this slug that matches any chain
    /// within the same namespace.
    ///
    /// For example, `eip155:8453:v2:exact` becomes `eip155:*:v2:exact`.
    #[must_use]
    pub fn as_wildcard(&self) -> Self {
        Self {
            chain_id: ChainId::new(self.chain_id.namespace(), "*"),
            x402_version: self.x402_version,
            name: self.name.clone(),
        }
    }

    /// Returns `true` if this slug uses a wildcard reference (`*`).
    #[must_use]
    pub fn is_wildcard(&self) -> bool {
        self.chain_id.reference() == "*"
    }
}

impl Display for SchemeHandlerSlug {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:v{}:{}",
            self.chain_id.namespace(),
            self.chain_id.reference(),
            self.x402_version,
            self.name
        )
    }
}

/// Registry of active scheme handlers.
///
/// Maps chain+scheme combinations to their handlers.
#[derive(Default)]
pub struct SchemeRegistry(HashMap<SchemeHandlerSlug, Box<dyn Facilitator>>);

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
    pub fn register<P: ChainProviderOps>(
        &mut self,
        blueprint: &dyn SchemeBlueprint<P>,
        provider: &P,
        config: Option<serde_json::Value>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let chain_id = provider.chain_id();
        let handler = blueprint.build(provider, config)?;
        let slug = SchemeHandlerSlug::new(
            chain_id,
            blueprint.x402_version(),
            blueprint.scheme().to_string(),
        );
        self.0.insert(slug, handler);
        Ok(())
    }

    /// Gets a handler by its slug.
    ///
    /// Performs a two-phase lookup:
    /// 1. Exact match on the full slug (namespace:reference:version:scheme)
    /// 2. Wildcard fallback on the namespace (namespace:*:version:scheme)
    ///
    /// This allows registering a single handler for an entire namespace
    /// (e.g., `eip155:*`) that serves all chains within it.
    #[must_use]
    pub fn by_slug(&self, slug: &SchemeHandlerSlug) -> Option<&dyn Facilitator> {
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
    pub fn register_for_namespace<P: ChainProviderOps>(
        &mut self,
        blueprint: &dyn SchemeBlueprint<P>,
        provider: &P,
        config: Option<serde_json::Value>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let handler = blueprint.build(provider, config)?;
        let namespace = provider.chain_id().namespace().to_owned();
        let slug = SchemeHandlerSlug::new(
            ChainId::new(namespace, "*"),
            blueprint.x402_version(),
            blueprint.scheme().to_string(),
        );
        self.0.insert(slug, handler);
        Ok(())
    }

    /// Returns an iterator over all registered handlers.
    pub fn values(&self) -> impl Iterator<Item = &dyn Facilitator> {
        self.0.values().map(|v| &**v)
    }
}
