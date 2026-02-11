//! Scheme blueprint and handler registries.
//!
//! [`SchemeBlueprints`] stores factories that can create handlers, while
//! [`SchemeRegistry`] holds the active handler instances keyed by chain+scheme.

use crate::chain::{ChainId, ChainProviderOps};

use std::collections::HashMap;
use std::fmt;
use std::fmt::{Debug, Display, Formatter};
use std::marker::PhantomData;

use super::X402SchemeId;
use super::handler::{SchemeHandler, SchemeHandlerBuilder};

/// Marker trait for types that are both identifiable and buildable.
///
/// This combines [`X402SchemeId`] and [`SchemeHandlerBuilder`] for
/// use in the blueprint registry.
pub trait SchemeBlueprint<P>: X402SchemeId + for<'a> SchemeHandlerBuilder<&'a P> {}
impl<T, P> SchemeBlueprint<P> for T where T: X402SchemeId + for<'a> SchemeHandlerBuilder<&'a P> {}

/// Registry of scheme blueprints (factories).
///
/// Register blueprints at startup, then use them to build handlers
/// via [`SchemeRegistry`].
///
/// # Type Parameters
///
/// - `P` - The chain provider type
#[derive(Default)]
pub struct SchemeBlueprints<P>(HashMap<String, Box<dyn SchemeBlueprint<P>>>, PhantomData<P>);

impl<P> Debug for SchemeBlueprints<P> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let slugs: Vec<String> = self.0.keys().cloned().collect();
        f.debug_tuple("SchemeBlueprints").field(&slugs).finish()
    }
}

impl<P> SchemeBlueprints<P> {
    /// Creates an empty blueprint registry.
    #[must_use]
    pub fn new() -> Self {
        Self(HashMap::new(), PhantomData)
    }

    /// Registers a blueprint and returns self for chaining.
    #[must_use]
    pub fn and_register<B: SchemeBlueprint<P> + 'static>(mut self, blueprint: B) -> Self {
        self.register(blueprint);
        self
    }

    /// Registers a scheme blueprint.
    pub fn register<B: SchemeBlueprint<P> + 'static>(&mut self, blueprint: B) {
        self.0.insert(blueprint.id(), Box::new(blueprint));
    }

    /// Gets a blueprint by its ID.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&dyn SchemeBlueprint<P>> {
        self.0.get(id).map(|v| &**v)
    }
}

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
pub struct SchemeRegistry(HashMap<SchemeHandlerSlug, Box<dyn SchemeHandler>>);

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
    pub fn by_slug(&self, slug: &SchemeHandlerSlug) -> Option<&dyn SchemeHandler> {
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
    pub fn values(&self) -> impl Iterator<Item = &dyn SchemeHandler> {
        self.0.values().map(|v| &**v)
    }
}
