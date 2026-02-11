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
//! The [`HookedFacilitator`] decorator wraps any [`Facilitator`](crate::Facilitator)
//! and applies registered hooks around its verify/settle calls, following the same
//! lifecycle pattern as the official x402 Go SDK.

use std::fmt::{self, Debug};
use std::future::Future;
use std::pin::Pin;

use crate::facilitator::Facilitator;
use crate::proto;

/// Decision returned by "before" hooks to control whether an operation proceeds.
///
/// Mirrors the official x402 `BeforeHookResult` / `FacilitatorBeforeHookResult`.
#[derive(Debug, Clone)]
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
///
/// Mirrors the official x402 `FacilitatorVerifyContext`.
pub struct VerifyHookContext {
    /// The raw verify request (contains payload + requirements as JSON).
    pub request: proto::VerifyRequest,
}

impl Debug for VerifyHookContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerifyHookContext")
            .field("request", &"<VerifyRequest>")
            .finish()
    }
}

/// Context passed to settle lifecycle hooks.
///
/// Provides access to the raw settle request.
///
/// Mirrors the official x402 `FacilitatorSettleContext`.
pub struct SettleHookContext {
    /// The raw settle request (same structure as verify request).
    pub request: proto::SettleRequest,
}

impl Debug for SettleHookContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SettleHookContext")
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
        _ctx: &'a VerifyHookContext,
    ) -> Pin<Box<dyn Future<Output = HookDecision> + Send + 'a>> {
        Box::pin(async { HookDecision::Continue })
    }

    /// Called after successful payment verification.
    ///
    /// Any error returned will be logged but will not affect the verification result.
    fn after_verify<'a>(
        &'a self,
        _ctx: &'a VerifyHookContext,
        _result: &'a proto::VerifyResponse,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }

    /// Called when payment verification fails.
    ///
    /// If a hook returns [`FailureRecovery::Recovered`], the provided `VerifyResponse`
    /// is returned instead of the error.
    fn on_verify_failure<'a>(
        &'a self,
        _ctx: &'a VerifyHookContext,
        _error: &'a (dyn std::error::Error + Send + Sync),
    ) -> Pin<Box<dyn Future<Output = FailureRecovery<proto::VerifyResponse>> + Send + 'a>> {
        Box::pin(async { FailureRecovery::Propagate })
    }

    /// Called before payment settlement.
    ///
    /// If any hook returns [`HookDecision::Abort`], settlement is skipped and
    /// an error is returned with the provided reason.
    fn before_settle<'a>(
        &'a self,
        _ctx: &'a SettleHookContext,
    ) -> Pin<Box<dyn Future<Output = HookDecision> + Send + 'a>> {
        Box::pin(async { HookDecision::Continue })
    }

    /// Called after successful payment settlement.
    ///
    /// Any error returned will be logged but will not affect the settlement result.
    fn after_settle<'a>(
        &'a self,
        _ctx: &'a SettleHookContext,
        _result: &'a proto::SettleResponse,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }

    /// Called when payment settlement fails.
    ///
    /// If a hook returns [`FailureRecovery::Recovered`], the provided `SettleResponse`
    /// is returned instead of the error.
    fn on_settle_failure<'a>(
        &'a self,
        _ctx: &'a SettleHookContext,
        _error: &'a (dyn std::error::Error + Send + Sync),
    ) -> Pin<Box<dyn Future<Output = FailureRecovery<proto::SettleResponse>> + Send + 'a>> {
        Box::pin(async { FailureRecovery::Propagate })
    }
}

/// Error type for [`HookedFacilitator`] that wraps inner facilitator errors
/// and adds hook-triggered abort errors.
#[derive(Debug, thiserror::Error)]
pub enum HookedFacilitatorError<E> {
    /// The inner facilitator returned an error.
    #[error(transparent)]
    Inner(E),
    /// A "before" hook aborted the operation.
    #[error("{reason}: {message}")]
    Aborted {
        /// Machine-readable abort reason.
        reason: String,
        /// Human-readable abort message.
        message: String,
    },
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
    F: Facilitator + Send + Sync,
    F::Error: std::error::Error + Send + Sync + 'static,
{
    type Error = HookedFacilitatorError<F::Error>;

    async fn verify(
        &self,
        request: proto::VerifyRequest,
    ) -> Result<proto::VerifyResponse, Self::Error> {
        let ctx = VerifyHookContext {
            request: request.clone(),
        };

        // Phase 1: Before hooks — first abort wins
        for hook in &self.hooks {
            if let HookDecision::Abort { reason, message } = hook.before_verify(&ctx).await {
                return Err(HookedFacilitatorError::Aborted { reason, message });
            }
        }

        // Phase 2: Execute inner facilitator
        match self.inner.verify(request).await {
            Ok(response) => {
                // Phase 3a: After hooks (fire-and-forget)
                for hook in &self.hooks {
                    hook.after_verify(&ctx, &response).await;
                }
                Ok(response)
            }
            Err(e) => {
                // Phase 3b: Failure hooks — first recovery wins
                for hook in &self.hooks {
                    if let FailureRecovery::Recovered(response) =
                        hook.on_verify_failure(&ctx, &e).await
                    {
                        return Ok(response);
                    }
                }
                Err(HookedFacilitatorError::Inner(e))
            }
        }
    }

    async fn settle(
        &self,
        request: proto::SettleRequest,
    ) -> Result<proto::SettleResponse, Self::Error> {
        let ctx = SettleHookContext {
            request: request.clone(),
        };

        // Phase 1: Before hooks — first abort wins
        for hook in &self.hooks {
            if let HookDecision::Abort { reason, message } = hook.before_settle(&ctx).await {
                return Err(HookedFacilitatorError::Aborted { reason, message });
            }
        }

        // Phase 2: Execute inner facilitator
        match self.inner.settle(request).await {
            Ok(response) => {
                // Phase 3a: After hooks (fire-and-forget)
                for hook in &self.hooks {
                    hook.after_settle(&ctx, &response).await;
                }
                Ok(response)
            }
            Err(e) => {
                // Phase 3b: Failure hooks — first recovery wins
                for hook in &self.hooks {
                    if let FailureRecovery::Recovered(response) =
                        hook.on_settle_failure(&ctx, &e).await
                    {
                        return Ok(response);
                    }
                }
                Err(HookedFacilitatorError::Inner(e))
            }
        }
    }

    async fn supported(&self) -> Result<proto::SupportedResponse, Self::Error> {
        self.inner
            .supported()
            .await
            .map_err(HookedFacilitatorError::Inner)
    }
}
