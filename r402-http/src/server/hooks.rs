//! Lifecycle hooks for the x402 payment gate.
//!
//! Hooks allow resource servers to intercept and customize the payment
//! verification and settlement lifecycle. This mirrors the Go SDK's
//! `server_hooks.go` design and aligns with [`r402::hooks::FacilitatorHooks`].
//!
//! ## Hook Lifecycle
//!
//! 1. **`before_*`** — Runs before the operation. Can abort with a reason.
//! 2. **Inner operation executes**
//! 3. **`after_*`** (on success) — Observes the result. Cannot affect the outcome.
//! 4. **`on_*_failure`** (on error) — Can recover with a substitute result.
//!
//! ## Usage
//!
//! Implement [`PaygateHooks`] with only the hooks you need — all methods
//! have default no-op implementations.

use r402::hooks::{FailureRecovery, HookDecision};
use r402::proto;
use std::future::Future;
use std::pin::Pin;

/// Context passed to verify lifecycle hooks.
#[derive(Debug, Clone)]
pub struct VerifyContext {
    /// The verify request about to be (or already) sent to the facilitator.
    pub request: proto::VerifyRequest,
}

/// Context passed to settle lifecycle hooks.
#[derive(Debug, Clone)]
pub struct SettleContext {
    /// The settle request about to be (or already) sent to the facilitator.
    pub request: proto::SettleRequest,
}

/// Lifecycle hooks for payment gate verify and settle operations.
///
/// All methods have default no-op implementations. Override only the hooks you
/// need. This trait is dyn-compatible for use in heterogeneous hook lists.
///
/// The hook lifecycle mirrors [`r402::hooks::FacilitatorHooks`]:
///
/// 1. **`before_*`** — Runs before the operation. Can abort with a reason.
/// 2. **Inner operation executes**
/// 3. **`after_*`** (on success) — Observes the result.
/// 4. **`on_*_failure`** (on error) — Can recover with a substitute result.
pub trait PaygateHooks: Send + Sync {
    /// Called before payment verification.
    ///
    /// If any hook returns [`HookDecision::Abort`], verification is skipped and
    /// an error is returned with the provided reason.
    fn before_verify<'a>(
        &'a self,
        _ctx: &'a VerifyContext,
    ) -> Pin<Box<dyn Future<Output = HookDecision> + Send + 'a>> {
        Box::pin(async { HookDecision::Continue })
    }

    /// Called after successful payment verification.
    fn after_verify<'a>(
        &'a self,
        _ctx: &'a VerifyContext,
        _result: &'a proto::VerifyResponse,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }

    /// Called when payment verification fails.
    ///
    /// If a hook returns [`FailureRecovery::Recovered`], the provided response
    /// is returned instead of the error.
    fn on_verify_failure<'a>(
        &'a self,
        _ctx: &'a VerifyContext,
        _error: &'a str,
    ) -> Pin<Box<dyn Future<Output = FailureRecovery<proto::VerifyResponse>> + Send + 'a>> {
        Box::pin(async { FailureRecovery::Propagate })
    }

    /// Called before payment settlement.
    ///
    /// If any hook returns [`HookDecision::Abort`], settlement is skipped and
    /// an error is returned with the provided reason.
    fn before_settle<'a>(
        &'a self,
        _ctx: &'a SettleContext,
    ) -> Pin<Box<dyn Future<Output = HookDecision> + Send + 'a>> {
        Box::pin(async { HookDecision::Continue })
    }

    /// Called after successful payment settlement.
    fn after_settle<'a>(
        &'a self,
        _ctx: &'a SettleContext,
        _result: &'a proto::SettleResponse,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }

    /// Called when payment settlement fails.
    ///
    /// If a hook returns [`FailureRecovery::Recovered`], the provided response
    /// is returned instead of the error.
    fn on_settle_failure<'a>(
        &'a self,
        _ctx: &'a SettleContext,
        _error: &'a str,
    ) -> Pin<Box<dyn Future<Output = FailureRecovery<proto::SettleResponse>> + Send + 'a>> {
        Box::pin(async { FailureRecovery::Propagate })
    }
}
