//! Lifecycle hooks for the x402 client payment creation pipeline.
//!
//! Hooks allow applications to intercept and customize the payment
//! creation lifecycle. This mirrors the Go SDK's `client_hooks.go` design,
//! using the same `HookDecision` / `FailureRecovery` enums as the
//! server-side hooks for a consistent API.
//!
//! ## Hook Lifecycle
//!
//! 1. **`before_payment_creation`** — Run before payment creation; can abort it.
//! 2. **Payment signing executes**
//! 3. **`after_payment_creation`** (on success) — Observes the result.
//! 4. **`on_payment_creation_failure`** (on error) — Can recover with substitute headers.
//!
//! ## Usage
//!
//! Implement [`ClientHooks`] with only the hooks you need — all methods
//! have default no-op implementations.

use http::HeaderMap;
use r402::hooks::{FailureRecovery, HookDecision};
use r402::proto;
use std::future::Future;
use std::pin::Pin;

/// Context passed to client payment creation lifecycle hooks.
#[derive(Debug, Clone)]
pub struct PaymentCreationContext {
    /// The parsed payment requirements from the 402 response.
    pub payment_required: proto::PaymentRequired,
}

/// Lifecycle hooks for client-side payment creation.
///
/// All methods have default no-op implementations. Override only the hooks you
/// need. This trait is dyn-compatible for use in heterogeneous hook lists.
///
/// The hook lifecycle mirrors [`r402::hooks::FacilitatorHooks`]:
///
/// 1. **`before_payment_creation`** — Can abort with [`HookDecision::Abort`].
/// 2. **Payment signing executes**
/// 3. **`after_payment_creation`** (on success) — Observes the signed headers.
/// 4. **`on_payment_creation_failure`** (on error) — Can recover with [`FailureRecovery::Recovered`].
pub trait ClientHooks: Send + Sync {
    /// Called before payment creation.
    ///
    /// If any hook returns [`HookDecision::Abort`], payment creation is skipped
    /// and the original 402 response is returned to the caller.
    fn before_payment_creation<'a>(
        &'a self,
        _ctx: &'a PaymentCreationContext,
    ) -> Pin<Box<dyn Future<Output = HookDecision> + Send + 'a>> {
        Box::pin(async { HookDecision::Continue })
    }

    /// Called after successful payment creation.
    ///
    /// Receives the signed payment headers. Cannot affect the outcome.
    fn after_payment_creation<'a>(
        &'a self,
        _ctx: &'a PaymentCreationContext,
        _headers: &'a HeaderMap,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }

    /// Called when payment creation fails.
    ///
    /// If a hook returns [`FailureRecovery::Recovered`], the provided headers
    /// replace the error.
    fn on_payment_creation_failure<'a>(
        &'a self,
        _ctx: &'a PaymentCreationContext,
        _error: &'a str,
    ) -> Pin<Box<dyn Future<Output = FailureRecovery<HeaderMap>> + Send + 'a>> {
        Box::pin(async { FailureRecovery::Propagate })
    }
}
