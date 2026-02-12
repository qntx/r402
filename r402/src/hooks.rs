//! Lifecycle hooks for x402 facilitator operations.
//!
//! This module provides the hook system that allows intercepting verify and settle
//! operations at three points in their lifecycle:
//!
//! - **Before**: Inspect or abort the operation before it executes
//! - **After**: Observe the result after a successful operation
//! - **On Failure**: Observe or recover from a failed operation
//!
//! # Architecture
//!
//! Hooks are defined via the [`FacilitatorHooks`] trait, which has default no-op
//! implementations for all methods. Implement only the hooks you need.
//!
//! The [`HookedFacilitator`] decorator wraps any [`Facilitator`]
//! and applies registered hooks around its verify/settle calls, following the same
//! lifecycle pattern as the official x402 Go SDK.

use std::fmt::{self, Debug};

use crate::facilitator::{BoxFuture, Facilitator, FacilitatorError};
use crate::proto;

/// Decision returned by "before" hooks to control whether an operation proceeds.
///
/// Mirrors the official x402 `BeforeHookResult` / `FacilitatorBeforeHookResult`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum HookDecision {
    /// Allow the operation to proceed normally.
    Continue,
    /// Abort the operation with the given reason and optional human-readable message.
    Abort {
        /// Machine-readable reason for aborting (e.g., `"kyt_blocked"`).
        reason: String,
        /// Optional human-readable message describing why the operation was aborted.
        message: String,
    },
}

/// Decision returned by "on failure" hooks to optionally recover from errors.
///
/// Mirrors the official x402 `VerifyFailureHookResult` / `SettleFailureHookResult`.
#[derive(Debug)]
#[non_exhaustive]
pub enum FailureRecovery<T> {
    /// The error was not recovered; propagate the original error.
    Propagate,
    /// The hook recovered from the failure with a substitute result.
    Recovered(T),
}

/// Context passed to verify lifecycle hooks.
///
/// Provides access to the raw verify request. The request contains the full
/// JSON payload and requirements, allowing hooks to inspect any field regardless
/// of protocol version.
#[derive(Clone)]
pub struct VerifyContext {
    /// The raw verify request (contains payload + requirements as JSON).
    pub request: proto::VerifyRequest,
}

impl Debug for VerifyContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerifyContext")
            .field("request", &"<VerifyRequest>")
            .finish()
    }
}

/// Context passed to settle lifecycle hooks.
///
/// Provides access to the raw settle request.
#[derive(Clone)]
pub struct SettleContext {
    /// The raw settle request (same structure as verify request).
    pub request: proto::SettleRequest,
}

impl Debug for SettleContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SettleContext")
            .field("request", &"<SettleRequest>")
            .finish()
    }
}

/// Lifecycle hooks for facilitator verify and settle operations.
///
/// All methods have default no-op implementations. Override only the hooks you
/// need. This trait is dyn-compatible for use in heterogeneous hook lists.
///
/// The hook lifecycle mirrors the official x402 Go SDK:
///
/// 1. **`before_*`** — Runs before the operation. Can abort with a reason.
/// 2. **Inner operation executes**
/// 3. **`after_*`** (on success) — Observes the result. Errors are logged, not propagated.
/// 4. **`on_*_failure`** (on error) — Can recover with a substitute result.
pub trait FacilitatorHooks: Send + Sync {
    /// Called before payment verification.
    ///
    /// If any hook returns [`HookDecision::Abort`], verification is skipped and
    /// an invalid `VerifyResponse` is returned with the provided reason.
    fn before_verify<'a>(
        &'a self,
        _ctx: &'a VerifyContext,
    ) -> BoxFuture<'a, HookDecision> {
        Box::pin(async { HookDecision::Continue })
    }

    /// Called after successful payment verification.
    ///
    /// Any error returned will be logged but will not affect the verification result.
    fn after_verify<'a>(
        &'a self,
        _ctx: &'a VerifyContext,
        _result: &'a proto::VerifyResponse,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    /// Called when payment verification fails.
    ///
    /// If a hook returns [`FailureRecovery::Recovered`], the provided `VerifyResponse`
    /// is returned instead of the error.
    fn on_verify_failure<'a>(
        &'a self,
        _ctx: &'a VerifyContext,
        _error: &'a FacilitatorError,
    ) -> BoxFuture<'a, FailureRecovery<proto::VerifyResponse>> {
        Box::pin(async { FailureRecovery::Propagate })
    }

    /// Called before payment settlement.
    ///
    /// If any hook returns [`HookDecision::Abort`], settlement is skipped and
    /// an error is returned with the provided reason.
    fn before_settle<'a>(
        &'a self,
        _ctx: &'a SettleContext,
    ) -> BoxFuture<'a, HookDecision> {
        Box::pin(async { HookDecision::Continue })
    }

    /// Called after successful payment settlement.
    ///
    /// Any error returned will be logged but will not affect the settlement result.
    fn after_settle<'a>(
        &'a self,
        _ctx: &'a SettleContext,
        _result: &'a proto::SettleResponse,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    /// Called when payment settlement fails.
    ///
    /// If a hook returns [`FailureRecovery::Recovered`], the provided `SettleResponse`
    /// is returned instead of the error.
    fn on_settle_failure<'a>(
        &'a self,
        _ctx: &'a SettleContext,
        _error: &'a FacilitatorError,
    ) -> BoxFuture<'a, FailureRecovery<proto::SettleResponse>> {
        Box::pin(async { FailureRecovery::Propagate })
    }
}

/// A facilitator decorator that applies lifecycle hooks around verify/settle operations.
///
/// Wraps any type implementing [`Facilitator`] and executes registered
/// [`FacilitatorHooks`] at the appropriate lifecycle points, following the
/// same pattern as the official x402 Go SDK's `x402Facilitator`.
///
/// Hooks are executed in registration order:
/// - **Before hooks**: First abort wins — remaining hooks are skipped.
/// - **After hooks**: All hooks run; errors are silently ignored.
/// - **Failure hooks**: First recovery wins — remaining hooks are skipped.
pub struct HookedFacilitator<F> {
    inner: F,
    hooks: Vec<Box<dyn FacilitatorHooks>>,
}

impl<F: Debug> Debug for HookedFacilitator<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HookedFacilitator")
            .field("inner", &self.inner)
            .field("hooks", &format!("[{} hooks]", self.hooks.len()))
            .finish()
    }
}

impl<F> HookedFacilitator<F> {
    /// Wraps a facilitator with hook support.
    pub fn new(inner: F) -> Self {
        Self {
            inner,
            hooks: Vec::new(),
        }
    }

    /// Registers a lifecycle hook. Hooks execute in registration order.
    #[must_use]
    pub fn with_hook(mut self, hook: impl FacilitatorHooks + 'static) -> Self {
        self.hooks.push(Box::new(hook));
        self
    }

    /// Adds a hook dynamically after construction.
    pub fn add_hook(&mut self, hook: impl FacilitatorHooks + 'static) {
        self.hooks.push(Box::new(hook));
    }

    /// Returns the number of registered hooks.
    #[must_use]
    pub fn hook_count(&self) -> usize {
        self.hooks.len()
    }

    /// Returns a reference to the inner facilitator.
    #[must_use]
    pub const fn inner(&self) -> &F {
        &self.inner
    }
}

impl<F> Facilitator for HookedFacilitator<F>
where
    F: Facilitator,
{
    fn verify(
        &self,
        request: proto::VerifyRequest,
    ) -> BoxFuture<'_, Result<proto::VerifyResponse, FacilitatorError>> {
        Box::pin(async move {
            let ctx = VerifyContext {
                request: request.clone(),
            };
            for hook in &self.hooks {
                if let HookDecision::Abort { reason, message } = hook.before_verify(&ctx).await {
                    return Err(FacilitatorError::Aborted { reason, message });
                }
            }
            match self.inner.verify(request).await {
                Ok(response) => {
                    for hook in &self.hooks {
                        hook.after_verify(&ctx, &response).await;
                    }
                    Ok(response)
                }
                Err(e) => {
                    for hook in &self.hooks {
                        if let FailureRecovery::Recovered(response) =
                            hook.on_verify_failure(&ctx, &e).await
                        {
                            return Ok(response);
                        }
                    }
                    Err(e)
                }
            }
        })
    }

    fn settle(
        &self,
        request: proto::SettleRequest,
    ) -> BoxFuture<'_, Result<proto::SettleResponse, FacilitatorError>> {
        Box::pin(async move {
            let ctx = SettleContext {
                request: request.clone(),
            };
            for hook in &self.hooks {
                if let HookDecision::Abort { reason, message } = hook.before_settle(&ctx).await {
                    return Err(FacilitatorError::Aborted { reason, message });
                }
            }
            match self.inner.settle(request).await {
                Ok(response) => {
                    for hook in &self.hooks {
                        hook.after_settle(&ctx, &response).await;
                    }
                    Ok(response)
                }
                Err(e) => {
                    for hook in &self.hooks {
                        if let FailureRecovery::Recovered(response) =
                            hook.on_settle_failure(&ctx, &e).await
                        {
                            return Ok(response);
                        }
                    }
                    Err(e)
                }
            }
        })
    }

    fn supported(
        &self,
    ) -> BoxFuture<'_, Result<proto::SupportedResponse, FacilitatorError>> {
        Box::pin(async move { self.inner.supported().await })
    }
}
