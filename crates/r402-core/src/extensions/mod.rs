//! Extension framework for x402 protocol extensions.
//!
//! Per the x402 v2 spec, an **extension** is a named piece of behaviour that
//! attaches additional fields to the wire types (`PaymentRequired.extensions`,
//! `PaymentPayload.extensions`, `VerifyResponse.extensions`, ...). Extensions
//! are orthogonal to schemes: any scheme may opt into any extension, and
//! extensions compose.
//!
//! # Architecture
//!
//! - [`Extension`] — the trait extensions implement. Uses AFIT (async fn in
//!   traits) for zero-cost static dispatch when the concrete type is known.
//! - [`DynExtension`] — object-safe erasure of [`Extension`] for heterogeneous
//!   registries.
//! - [`ExtensionRegistry`] — orders + indexes extensions for a facilitator /
//!   paygate.
//! - [`AdvertiseContext`] / [`VerifyContext`] / [`SettleContext`] — contexts
//!   passed to each hook, carrying immutable borrowed views of the wire types.
//!
//! # Built-in Extensions
//!
//! Feature-gated:
//!
//! - `ext-bazaar` — [`bazaar::BazaarExtension`] for resource discovery
//! - `ext-payment-id` — [`payment_id::PaymentIdentifierExtension`]
//! - `ext-agent-identity` — [`agent_identity::AgentIdentityExtension`]
//!   (ERC-8004 agent attribution)
//! - `ext-eip2612` — [`eip2612::Eip2612GasSponsoringExtension`]
//!   (server advertise; EVM payload types in `r402-evm`)
//! - `ext-erc20-approval` — [`erc20_approval::Erc20ApprovalGasSponsoringExtension`]
//!
//! # Implementing Your Own
//!
//! ```no_run
//! use std::future::Future;
//! use r402_core::extensions::{Extension, AdvertiseContext, VerifyContext};
//! use r402_core::wire::ExtensionEntry;
//!
//! struct MyExt;
//!
//! impl Extension for MyExt {
//!     fn id(&self) -> &'static str { "my-ext" }
//!     fn advertise(&self, _ctx: &AdvertiseContext<'_>) -> Option<ExtensionEntry> {
//!         Some(ExtensionEntry::info(serde_json::json!({"version": 1})))
//!     }
//! }
//! ```

use std::collections::HashMap;
use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use compact_str::CompactString;

use crate::wire::{
    ExtensionEntry, Extensions, PaymentPayload, PaymentRequirements, SettleResponse, VerifyResponse,
};

#[cfg(feature = "ext-agent-identity")]
#[cfg_attr(docsrs, doc(cfg(feature = "ext-agent-identity")))]
pub mod agent_identity;

#[cfg(feature = "ext-bazaar")]
#[cfg_attr(docsrs, doc(cfg(feature = "ext-bazaar")))]
pub mod bazaar;

#[cfg(feature = "ext-eip2612")]
#[cfg_attr(docsrs, doc(cfg(feature = "ext-eip2612")))]
pub mod eip2612;

#[cfg(feature = "ext-erc20-approval")]
#[cfg_attr(docsrs, doc(cfg(feature = "ext-erc20-approval")))]
pub mod erc20_approval;

#[cfg(feature = "ext-payment-id")]
#[cfg_attr(docsrs, doc(cfg(feature = "ext-payment-id")))]
pub mod payment_id;

/// Boxed future type alias used by [`DynExtension`].
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Context passed to [`Extension::advertise`].
#[derive(Debug)]
#[non_exhaustive]
pub struct AdvertiseContext<'a> {
    /// The requirement entry being advertised (if any). `None` when the
    /// extension is being advertised as a top-level `PaymentRequired.extensions`
    /// entry rather than per-requirement.
    pub requirement: Option<&'a PaymentRequirements>,
}

/// Context passed to [`Extension::on_verify`].
#[non_exhaustive]
pub struct VerifyContext<'a> {
    /// The fully-decoded payment payload (as generic JSON, since per-scheme
    /// types are unknown at this level). Use `serde_json::from_value` if the
    /// extension needs scheme-specific fields.
    pub payload: &'a PaymentPayload<serde_json::Value, serde_json::Value>,
    /// The matched requirements.
    pub requirements: &'a PaymentRequirements,
    /// The in-flight verify response (mutable so extensions may observe the
    /// preliminary outcome before returning their own payload).
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
    /// The fully-decoded payment payload.
    pub payload: &'a PaymentPayload<serde_json::Value, serde_json::Value>,
    /// The matched requirements.
    pub requirements: &'a PaymentRequirements,
    /// The in-flight settle response.
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
/// Each extension has a stable string ID that is used as the key in the
/// `extensions` map on the wire. Extensions opt into lifecycle hooks by
/// overriding default-noop methods.
///
/// **Static dispatch is preferred** — depend on concrete `impl Extension`
/// types whenever possible for zero overhead. Use [`DynExtension`] only
/// when a heterogeneous collection is unavoidable.
pub trait Extension: Send + Sync {
    /// Stable extension identifier (e.g. `"bazaar"`, `"payment-identifier"`).
    fn id(&self) -> &'static str;

    /// Called when the seller assembles a 402 response. Return an entry to
    /// attach to `PaymentRequired.extensions` (or the per-requirement
    /// extensions, if [`AdvertiseContext::requirement`] is `Some`).
    fn advertise(&self, _ctx: &AdvertiseContext<'_>) -> Option<ExtensionEntry> {
        None
    }

    /// Called by the facilitator after `verify`. Return an entry to attach
    /// to the outgoing `VerifyResponse.extensions` map.
    fn on_verify<'a>(
        &'a self,
        _ctx: &'a VerifyContext<'a>,
    ) -> impl Future<Output = Option<ExtensionEntry>> + Send + 'a {
        async { None }
    }

    /// Called by the facilitator after `settle`. Return an entry to attach
    /// to the outgoing `SettleResponse.extensions` map.
    fn on_settle<'a>(
        &'a self,
        _ctx: &'a SettleContext<'a>,
    ) -> impl Future<Output = Option<ExtensionEntry>> + Send + 'a {
        async { None }
    }
}

/// Dyn-compatible erasure of [`Extension`] used by [`ExtensionRegistry`].
///
/// Users rarely implement this directly; the blanket `impl<T: Extension>`
/// below turns any `Extension` into a `DynExtension`.
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

/// Ordered collection of extensions attached to a facilitator or paygate.
///
/// Stored as `Vec<Arc<dyn DynExtension>>` so extensions can be shared across
/// clones. Lookups by ID go through a parallel `HashMap`.
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
    /// Constructs an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an extension. Later registrations of the same id overwrite
    /// earlier ones (idempotent insert).
    pub fn register<E: Extension + 'static>(&mut self, extension: E) {
        let id = extension.id();
        let arc: Arc<dyn DynExtension> = Arc::new(extension);
        // de-duplicate by id
        self.ordered.retain(|existing| existing.id() != id);
        self.ordered.push(Arc::clone(&arc));
        let _ = self.by_id.insert(CompactString::from(id), arc);
    }

    /// Looks up an extension by its stable id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&dyn DynExtension> {
        self.by_id.get(id).map(AsRef::as_ref)
    }

    /// Iterates over all registered extensions in registration order.
    pub fn iter(&self) -> impl Iterator<Item = &dyn DynExtension> {
        self.ordered.iter().map(AsRef::as_ref)
    }

    /// Returns `true` when no extensions are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ordered.is_empty()
    }

    /// Number of registered extensions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ordered.len()
    }

    /// Builds a wire-level [`Extensions`] block for seller advertising.
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

    /// Builds a wire-level [`Extensions`] block by collecting each extension's
    /// `on_verify` return value.
    pub async fn collect_verify(&self, ctx: &VerifyContext<'_>) -> Extensions {
        let mut out = Extensions::new();
        for ext in self.iter() {
            if let Some(entry) = ext.on_verify(ctx).await {
                out.insert(ext.id(), entry);
            }
        }
        out
    }

    /// Builds a wire-level [`Extensions`] block by collecting each extension's
    /// `on_settle` return value.
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
