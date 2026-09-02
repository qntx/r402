//! Extension trait and registry.
//!
//! An extension attaches extra fields to `PaymentRequired.extensions`,
//! `PaymentPayload.extensions`, and verify/settle `extensions`. Facilitator
//! sidechannel outcomes live on response variants as `extension_responses`
//! and are never merged into `extensions`.

use std::collections::HashMap;
use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use compact_str::CompactString;

use crate::payment::{
    ExtensionEntry, Extensions, PaymentPayload, PaymentRequirements, SettleResponse, VerifyResponse,
};

/// Boxed future used by [`DynExtension`].
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Context passed to [`Extension::advertise`].
#[derive(Debug)]
#[non_exhaustive]
pub struct AdvertiseContext<'a> {
    /// Requirement being advertised. `None` for top-level `PaymentRequired.extensions`.
    pub requirement: Option<&'a PaymentRequirements>,
}

/// Context passed to [`Extension::on_verify`].
#[non_exhaustive]
pub struct VerifyContext<'a> {
    /// Decoded payment payload as generic JSON.
    pub payload: &'a PaymentPayload<serde_json::Value, serde_json::Value>,
    /// Matched requirements.
    pub requirements: &'a PaymentRequirements,
    /// In-flight verify response.
    pub response: &'a VerifyResponse,
}

impl Debug for VerifyContext<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerifyContext")
            .field("requirements", &self.requirements)
            .finish_non_exhaustive()
    }
}

/// Context passed to [`Extension::on_settle`].
#[non_exhaustive]
pub struct SettleContext<'a> {
    /// Decoded payment payload.
    pub payload: &'a PaymentPayload<serde_json::Value, serde_json::Value>,
    /// Matched requirements.
    pub requirements: &'a PaymentRequirements,
    /// In-flight settle response.
    pub response: &'a SettleResponse,
}

impl Debug for SettleContext<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SettleContext")
            .field("requirements", &self.requirements)
            .finish_non_exhaustive()
    }
}

/// A single x402 protocol extension.
///
/// Prefer static `impl Extension`. Use [`DynExtension`] only for a
/// heterogeneous collection.
pub trait Extension: Send + Sync {
    /// Stable extension identifier (`"bazaar"`, `"payment-identifier"`, …).
    fn id(&self) -> &'static str;

    /// Called when assembling a 402 response.
    fn advertise(&self, _ctx: &AdvertiseContext<'_>) -> Option<ExtensionEntry> {
        None
    }

    /// Called after facilitator `verify`. Return value goes on
    /// `VerifyResponse.extensions`, not `extension_responses`.
    fn on_verify<'a>(
        &'a self,
        _ctx: &'a VerifyContext<'a>,
    ) -> impl Future<Output = Option<ExtensionEntry>> + Send + 'a {
        std::future::ready(None)
    }

    /// Called after facilitator `settle`. Return value goes on
    /// `SettleResponse.extensions`, not `extension_responses`.
    fn on_settle<'a>(
        &'a self,
        _ctx: &'a SettleContext<'a>,
    ) -> impl Future<Output = Option<ExtensionEntry>> + Send + 'a {
        std::future::ready(None)
    }
}

/// Object-safe erasure of [`Extension`].
pub trait DynExtension: Send + Sync {
    /// Stable identifier.
    fn id(&self) -> &'static str;

    /// See [`Extension::advertise`].
    fn advertise(&self, ctx: &AdvertiseContext<'_>) -> Option<ExtensionEntry>;

    /// See [`Extension::on_verify`].
    fn on_verify<'a>(&'a self, ctx: &'a VerifyContext<'a>)
    -> BoxFuture<'a, Option<ExtensionEntry>>;

    /// See [`Extension::on_settle`].
    fn on_settle<'a>(&'a self, ctx: &'a SettleContext<'a>)
    -> BoxFuture<'a, Option<ExtensionEntry>>;
}

impl<T: Extension + ?Sized> DynExtension for T {
    fn id(&self) -> &'static str {
        <Self as Extension>::id(self)
    }

    fn advertise(&self, ctx: &AdvertiseContext<'_>) -> Option<ExtensionEntry> {
        <Self as Extension>::advertise(self, ctx)
    }

    fn on_verify<'a>(
        &'a self,
        ctx: &'a VerifyContext<'a>,
    ) -> BoxFuture<'a, Option<ExtensionEntry>> {
        Box::pin(<Self as Extension>::on_verify(self, ctx))
    }

    fn on_settle<'a>(
        &'a self,
        ctx: &'a SettleContext<'a>,
    ) -> BoxFuture<'a, Option<ExtensionEntry>> {
        Box::pin(<Self as Extension>::on_settle(self, ctx))
    }
}

/// Ordered collection of extensions.
#[derive(Clone, Default)]
pub struct ExtensionRegistry {
    ordered: Vec<Arc<dyn DynExtension>>,
    by_id: HashMap<CompactString, Arc<dyn DynExtension>>,
}

impl Debug for ExtensionRegistry {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let ids: Vec<&str> = self.ordered.iter().map(|e| e.id()).collect();
        f.debug_tuple("ExtensionRegistry").field(&ids).finish()
    }
}

impl ExtensionRegistry {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an extension. Same id overwrites the previous entry.
    pub fn register<E: Extension + 'static>(&mut self, extension: E) {
        let id = extension.id();
        let arc: Arc<dyn DynExtension> = Arc::new(extension);
        self.ordered.retain(|existing| existing.id() != id);
        self.ordered.push(Arc::clone(&arc));
        let _ = self.by_id.insert(CompactString::from(id), arc);
    }

    /// Looks up an extension by stable id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&dyn DynExtension> {
        self.by_id.get(id).map(AsRef::as_ref)
    }

    /// Registered extensions in registration order.
    pub fn iter(&self) -> impl Iterator<Item = &dyn DynExtension> {
        self.ordered.iter().map(AsRef::as_ref)
    }

    /// Whether no extensions are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ordered.is_empty()
    }

    /// Number of registered extensions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ordered.len()
    }

    /// Builds a wire [`Extensions`] block for seller advertising.
    #[must_use]
    pub fn advertise(&self, ctx: &AdvertiseContext<'_>) -> Extensions {
        let mut out = Extensions::new();
        for ext in self.iter() {
            if let Some(entry) = ext.advertise(ctx) {
                out.insert(ext.id(), entry);
            }
        }
        out
    }

    /// Collects each extension's `on_verify` return value.
    pub async fn collect_verify(&self, ctx: &VerifyContext<'_>) -> Extensions {
        let mut out = Extensions::new();
        for ext in self.iter() {
            if let Some(entry) = ext.on_verify(ctx).await {
                out.insert(ext.id(), entry);
            }
        }
        out
    }

    /// Collects each extension's `on_settle` return value.
    pub async fn collect_settle(&self, ctx: &SettleContext<'_>) -> Extensions {
        let mut out = Extensions::new();
        for ext in self.iter() {
            if let Some(entry) = ext.on_settle(ctx).await {
                out.insert(ext.id(), entry);
            }
        }
        out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "unit tests panic on assertion failure")]
mod tests {
    use serde_json::json;

    use super::*;

    struct StubExt(&'static str, serde_json::Value);

    impl Extension for StubExt {
        fn id(&self) -> &'static str {
            self.0
        }

        fn advertise(&self, _: &AdvertiseContext<'_>) -> Option<ExtensionEntry> {
            Some(ExtensionEntry::info(self.1.clone()))
        }
    }

    #[test]
    fn register_and_lookup() {
        let mut registry = ExtensionRegistry::new();
        registry.register(StubExt("bazaar", json!({"registered": true})));
        registry.register(StubExt("other", json!({"x": 1})));
        assert_eq!(registry.len(), 2);
        assert!(registry.get("bazaar").is_some());
        assert!(registry.get("missing").is_none());
    }

    #[test]
    fn duplicate_registration_overwrites() {
        let mut registry = ExtensionRegistry::new();
        registry.register(StubExt("x", json!(1)));
        registry.register(StubExt("x", json!(2)));
        assert_eq!(registry.len(), 1);
        let ctx = AdvertiseContext { requirement: None };
        let ext = registry.advertise(&ctx);
        let val = ext.get("x").unwrap().as_info().unwrap();
        assert_eq!(val, &json!(2));
    }

    #[test]
    fn advertise_emits_every_entry() {
        let mut registry = ExtensionRegistry::new();
        registry.register(StubExt("a", json!("A")));
        registry.register(StubExt("b", json!("B")));
        let ctx = AdvertiseContext { requirement: None };
        let ext = registry.advertise(&ctx);
        assert_eq!(ext.len(), 2);
    }
}
