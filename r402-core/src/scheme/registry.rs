//! Scheme handler builder, blueprint trait, and handler registry.
//!
//! [`SchemeBuilder`] defines how to construct a [`Facilitator`] from a
//! chain provider. [`SchemeBlueprint`] combines identity
//! ([`super::SchemeId`]) with building capability so the registry can
//! create handlers in a single call.
//!
//! [`SchemeRegistry`] stores the active handlers keyed by
//! [`SchemeSlug`] (chain + scheme name) and itself implements
//! [`Facilitator`], dispatching to the appropriate handler per request.

use std::collections::HashMap;
use std::fmt::{self, Debug, Display, Formatter};

use compact_str::CompactString;

use super::SchemeId;
use crate::chain::{ChainId, ChainProvider};
use crate::error::FacilitatorError;
use crate::facilitator::{DynFacilitator, Facilitator};
use crate::wire::{
    SettleRequest, SettleResponse, SupportedResponse, VerifyRequest, VerifyResponse,
};

/// Builds facilitator instances from a chain provider.
///
/// Each chain crate ships a blueprint per scheme (e.g.
/// `ExactEvmBlueprint`, `UptoEvmBlueprint`) that knows how to construct a
/// `Box<dyn DynFacilitator>` given a chain provider reference and optional
/// JSON configuration.
pub trait SchemeBuilder<P> {
    /// Builds a facilitator for the given chain provider.
    ///
    /// # Errors
    ///
    /// Returns an error if the facilitator cannot be built.
    fn build(
        &self,
        provider: P,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Marker trait combining identity ([`SchemeId`]) with build capability.
///
/// Blanket-implemented for any type that is both.
pub trait SchemeBlueprint<P>: SchemeId + for<'a> SchemeBuilder<&'a P> {}
impl<T, P> SchemeBlueprint<P> for T where T: SchemeId + for<'a> SchemeBuilder<&'a P> {}

/// Unique identifier for a scheme handler instance.
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct SchemeSlug {
    /// Chain this handler operates on.
    pub chain_id: ChainId,
    /// Scheme name (e.g. `"exact"`, `"upto"`).
    pub name: CompactString,
}

impl SchemeSlug {
    /// Constructs a new slug.
    #[must_use]
    pub const fn new(chain_id: ChainId, name: CompactString) -> Self {
        Self { chain_id, name }
    }

    /// Returns a wildcard version with the reference replaced by `*`.
    #[must_use]
    pub fn as_wildcard(&self) -> Self {
        Self {
            chain_id: ChainId::new(self.chain_id.namespace(), "*"),
            name: self.name.clone(),
        }
    }

    /// Returns `true` when the reference is the wildcard `*`.
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
            self.name,
        )
    }
}

/// Registry of active scheme handlers.
#[derive(Default)]
pub struct SchemeRegistry {
    handlers: HashMap<SchemeSlug, Box<dyn DynFacilitator>>,
}

impl Debug for SchemeRegistry {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let slugs: Vec<String> = self.handlers.keys().map(ToString::to_string).collect();
        f.debug_tuple("SchemeRegistry").field(&slugs).finish()
    }
}

impl SchemeRegistry {
    /// Constructs an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a handler for the given blueprint and provider.
    ///
    /// # Errors
    ///
    /// Returns an error if the blueprint fails to build.
    pub fn register<P: ChainProvider>(
        &mut self,
        blueprint: &dyn SchemeBlueprint<P>,
        provider: &P,
        config: Option<serde_json::Value>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let chain_id = provider.chain_id();
        let handler = blueprint.build(provider, config)?;
        let slug = SchemeSlug::new(chain_id, CompactString::from(blueprint.scheme()));
        let _ = self.handlers.insert(slug, handler);
        Ok(())
    }

    /// Registers a single handler for an entire CAIP-2 namespace.
    ///
    /// # Errors
    ///
    /// Returns an error if the blueprint fails to build.
    pub fn register_for_namespace<P: ChainProvider>(
        &mut self,
        blueprint: &dyn SchemeBlueprint<P>,
        provider: &P,
        config: Option<serde_json::Value>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let handler = blueprint.build(provider, config)?;
        let namespace = provider.chain_id().namespace().to_owned();
        let slug = SchemeSlug::new(
            ChainId::new(namespace, "*"),
            CompactString::from(blueprint.scheme()),
        );
        let _ = self.handlers.insert(slug, handler);
        Ok(())
    }

    /// Looks up a handler with wildcard fallback.
    #[must_use]
    pub fn by_slug(&self, slug: &SchemeSlug) -> Option<&dyn DynFacilitator> {
        self.handlers
            .get(slug)
            .or_else(|| self.handlers.get(&slug.as_wildcard()))
            .map(|h| &**h)
    }

    /// Iterates over registered handlers.
    pub fn values(&self) -> impl Iterator<Item = &dyn DynFacilitator> {
        self.handlers.values().map(|v| &**v)
    }

    async fn collect_supported(&self) -> SupportedResponse {
        let mut kinds = Vec::new();
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::new();
        for handler in self.values() {
            let Ok(mut resp) = handler.supported().await else {
                continue;
            };
            kinds.append(&mut resp.kinds);
            for (family, addrs) in resp.signers {
                signers.entry(family).or_default().extend(addrs);
            }
        }
        for addrs in signers.values_mut() {
            addrs.sort_unstable();
            addrs.dedup();
        }
        SupportedResponse {
            kinds,
            extensions: Vec::new(),
            signers,
        }
    }

    fn require_handler(
        &self,
        slug: Option<SchemeSlug>,
    ) -> Result<&dyn DynFacilitator, FacilitatorError> {
        slug.and_then(|s| self.by_slug(&s))
            .ok_or_else(|| FacilitatorError::aborted(
                "no_facilitator_for_network",
                "no handler registered for this payment scheme",
            ))
    }
}

impl Facilitator for SchemeRegistry {
    async fn verify(
        &self,
        request: VerifyRequest,
    ) -> Result<VerifyResponse, FacilitatorError> {
        let handler = self.require_handler(request.scheme_slug())?;
        handler.verify(request).await
    }

    async fn settle(
        &self,
        request: SettleRequest,
    ) -> Result<SettleResponse, FacilitatorError> {
        let handler = self.require_handler(request.scheme_slug())?;
        handler.settle(request).await
    }

    async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
        Ok(self.collect_supported().await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Extensions;

    struct StubFacilitator(CompactString);

    impl Facilitator for StubFacilitator {
        async fn verify(
            &self,
            _request: VerifyRequest,
        ) -> Result<VerifyResponse, FacilitatorError> {
            Ok(VerifyResponse::valid(self.0.clone()))
        }

        async fn settle(
            &self,
            _request: SettleRequest,
        ) -> Result<SettleResponse, FacilitatorError> {
            Ok(SettleResponse::Success {
                payer: "0x".into(),
                transaction: "0x".into(),
                network: "eip155:1".into(),
                amount: None,
                extensions: Extensions::new(),
            })
        }

        async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
            Ok(SupportedResponse::default())
        }
    }

    fn make_verify(network: &str, scheme: &str) -> VerifyRequest {
        serde_json::json!({
            "x402Version": 2,
            "paymentPayload": {
                "accepted": { "network": network, "scheme": scheme }
            },
            "paymentRequirements": { "network": network }
        })
        .into()
    }

    #[test]
    fn slug_display_format() {
        let slug = SchemeSlug::new(ChainId::new("eip155", "8453"), "exact".into());
        assert_eq!(slug.to_string(), "eip155:8453:exact");
    }

    #[test]
    fn slug_wildcard_conversion() {
        let slug = SchemeSlug::new(ChainId::new("eip155", "8453"), "exact".into());
        assert!(!slug.is_wildcard());
        let wild = slug.as_wildcard();
        assert!(wild.is_wildcard());
    }

    #[test]
    fn by_slug_exact_hit_and_miss() {
        let mut registry = SchemeRegistry::new();
        let slug = SchemeSlug::new(ChainId::new("eip155", "1"), "exact".into());
        let _ = registry
            .handlers
            .insert(slug.clone(), Box::new(StubFacilitator("eth".into())));
        assert!(registry.by_slug(&slug).is_some());

        let miss = SchemeSlug::new(ChainId::new("eip155", "999"), "exact".into());
        assert!(registry.by_slug(&miss).is_none());
    }

    #[test]
    fn by_slug_wildcard_fallback() {
        let mut registry = SchemeRegistry::new();
        let wild = SchemeSlug::new(ChainId::new("eip155", "*"), "exact".into());
        let _ = registry
            .handlers
            .insert(wild, Box::new(StubFacilitator("evm".into())));

        let query = SchemeSlug::new(ChainId::new("eip155", "42161"), "exact".into());
        assert!(registry.by_slug(&query).is_some());
    }

    #[test]
    fn by_slug_exact_takes_priority() {
        let mut registry = SchemeRegistry::new();
        let wild = SchemeSlug::new(ChainId::new("eip155", "*"), "exact".into());
        let exact = SchemeSlug::new(ChainId::new("eip155", "1"), "exact".into());
        let _ = registry
            .handlers
            .insert(wild, Box::new(StubFacilitator("wild".into())));
        let _ = registry
            .handlers
            .insert(exact.clone(), Box::new(StubFacilitator("exact".into())));
        assert!(registry.by_slug(&exact).is_some());
    }

    fn stub(tag: &str) -> Box<dyn DynFacilitator> {
        Box::new(StubFacilitator(tag.into()))
    }

    #[tokio::test]
    async fn dispatch_verify_routes_to_correct_handler() {
        let mut registry = SchemeRegistry::new();
        let slug = SchemeSlug::new(ChainId::new("eip155", "8453"), "exact".into());
        let _ = registry.handlers.insert(slug, stub("base_handler"));
        let resp = Facilitator::verify(&registry, make_verify("eip155:8453", "exact"))
            .await
            .unwrap();
        assert!(resp.is_valid());
    }

    #[tokio::test]
    async fn dispatch_no_handler_returns_aborted() {
        let registry = SchemeRegistry::new();
        let err = Facilitator::verify(&registry, make_verify("eip155:999", "exact"))
            .await
            .unwrap_err();
        assert!(matches!(err, FacilitatorError::Aborted { reason, .. }
            if reason == "no_facilitator_for_network"));
    }

    #[tokio::test]
    async fn dispatch_malformed_request_returns_aborted() {
        let registry = SchemeRegistry::new();
        let req: VerifyRequest = serde_json::json!({}).into();
        let err = Facilitator::verify(&registry, req).await.unwrap_err();
        assert!(matches!(err, FacilitatorError::Aborted { .. }));
    }

    #[tokio::test]
    async fn collect_supported_empty_registry() {
        let registry = SchemeRegistry::new();
        let resp = Facilitator::supported(&registry).await.unwrap();
        assert!(resp.kinds.is_empty());
        assert!(resp.signers.is_empty());
    }
}
