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
    fn before_verify<'a>(&'a self, _ctx: &'a VerifyContext) -> BoxFuture<'a, HookDecision> {
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
    fn before_settle<'a>(&'a self, _ctx: &'a SettleContext) -> BoxFuture<'a, HookDecision> {
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

impl<F: Send + Sync> HookedFacilitator<F> {
    async fn run_before_verify_hooks(&self, ctx: &VerifyContext) -> Result<(), FacilitatorError> {
        for hook in &self.hooks {
            if let HookDecision::Abort { reason, message } = hook.before_verify(ctx).await {
                return Err(FacilitatorError::Aborted { reason, message });
            }
        }
        Ok(())
    }

    async fn run_after_verify_hooks(&self, ctx: &VerifyContext, response: &proto::VerifyResponse) {
        for hook in &self.hooks {
            hook.after_verify(ctx, response).await;
        }
    }

    async fn run_on_verify_failure_hooks(
        &self,
        ctx: &VerifyContext,
        error: &FacilitatorError,
    ) -> Option<proto::VerifyResponse> {
        for hook in &self.hooks {
            if let FailureRecovery::Recovered(response) = hook.on_verify_failure(ctx, error).await {
                return Some(response);
            }
        }
        None
    }

    async fn run_before_settle_hooks(&self, ctx: &SettleContext) -> Result<(), FacilitatorError> {
        for hook in &self.hooks {
            if let HookDecision::Abort { reason, message } = hook.before_settle(ctx).await {
                return Err(FacilitatorError::Aborted { reason, message });
            }
        }
        Ok(())
    }

    async fn run_after_settle_hooks(&self, ctx: &SettleContext, response: &proto::SettleResponse) {
        for hook in &self.hooks {
            hook.after_settle(ctx, response).await;
        }
    }

    async fn run_on_settle_failure_hooks(
        &self,
        ctx: &SettleContext,
        error: &FacilitatorError,
    ) -> Option<proto::SettleResponse> {
        for hook in &self.hooks {
            if let FailureRecovery::Recovered(response) = hook.on_settle_failure(ctx, error).await {
                return Some(response);
            }
        }
        None
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
            self.run_before_verify_hooks(&ctx).await?;
            match self.inner.verify(request).await {
                Ok(response) => {
                    self.run_after_verify_hooks(&ctx, &response).await;
                    Ok(response)
                }
                Err(e) => self.run_on_verify_failure_hooks(&ctx, &e).await.ok_or(e),
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
            self.run_before_settle_hooks(&ctx).await?;
            match self.inner.settle(request).await {
                Ok(response) => {
                    self.run_after_settle_hooks(&ctx, &response).await;
                    Ok(response)
                }
                Err(e) => self.run_on_settle_failure_hooks(&ctx, &e).await.ok_or(e),
            }
        })
    }

    fn supported(&self) -> BoxFuture<'_, Result<proto::SupportedResponse, FacilitatorError>> {
        Box::pin(async move { self.inner.supported().await })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct MockFacilitator {
        fail: bool,
    }

    impl MockFacilitator {
        fn ok() -> Self {
            Self { fail: false }
        }
        fn failing() -> Self {
            Self { fail: true }
        }
        fn mock_verify(&self) -> Result<proto::VerifyResponse, FacilitatorError> {
            if self.fail {
                Err(FacilitatorError::OnchainFailure("mock".into()))
            } else {
                Ok(proto::VerifyResponse::valid("0xPAYER".into()))
            }
        }
        fn mock_settle(&self) -> Result<proto::SettleResponse, FacilitatorError> {
            if self.fail {
                Err(FacilitatorError::OnchainFailure("mock".into()))
            } else {
                Ok(proto::SettleResponse::Success {
                    payer: "0xPAYER".into(),
                    transaction: "0xTX".into(),
                    network: "eip155:1".into(),
                    extensions: None,
                })
            }
        }
    }

    impl Facilitator for MockFacilitator {
        fn verify(
            &self,
            _request: proto::VerifyRequest,
        ) -> BoxFuture<'_, Result<proto::VerifyResponse, FacilitatorError>> {
            Box::pin(async move { self.mock_verify() })
        }

        fn settle(
            &self,
            _request: proto::SettleRequest,
        ) -> BoxFuture<'_, Result<proto::SettleResponse, FacilitatorError>> {
            Box::pin(async move { self.mock_settle() })
        }

        fn supported(&self) -> BoxFuture<'_, Result<proto::SupportedResponse, FacilitatorError>> {
            Box::pin(async { Ok(proto::SupportedResponse::default()) })
        }
    }

    struct AbortVerifyHook;
    impl FacilitatorHooks for AbortVerifyHook {
        fn before_verify<'a>(&'a self, _: &'a VerifyContext) -> BoxFuture<'a, HookDecision> {
            Box::pin(async {
                HookDecision::Abort {
                    reason: "blocked".into(),
                    message: "test".into(),
                }
            })
        }
    }

    struct AbortSettleHook;
    impl FacilitatorHooks for AbortSettleHook {
        fn before_settle<'a>(&'a self, _: &'a SettleContext) -> BoxFuture<'a, HookDecision> {
            Box::pin(async {
                HookDecision::Abort {
                    reason: "blocked".into(),
                    message: "test".into(),
                }
            })
        }
    }

    struct RecoverVerifyHook;
    impl FacilitatorHooks for RecoverVerifyHook {
        fn on_verify_failure<'a>(
            &'a self,
            _: &'a VerifyContext,
            _: &'a FacilitatorError,
        ) -> BoxFuture<'a, FailureRecovery<proto::VerifyResponse>> {
            Box::pin(async {
                FailureRecovery::Recovered(proto::VerifyResponse::valid("0xREC".into()))
            })
        }
    }

    struct RecoverSettleHook;
    impl FacilitatorHooks for RecoverSettleHook {
        fn on_settle_failure<'a>(
            &'a self,
            _: &'a SettleContext,
            _: &'a FacilitatorError,
        ) -> BoxFuture<'a, FailureRecovery<proto::SettleResponse>> {
            Box::pin(async {
                FailureRecovery::Recovered(proto::SettleResponse::Success {
                    payer: "0xREC".into(),
                    transaction: "0xREC_TX".into(),
                    network: "eip155:1".into(),
                    extensions: None,
                })
            })
        }
    }

    struct NoopHook;
    impl FacilitatorHooks for NoopHook {}

    static SECOND_HOOK_CALLS: AtomicUsize = AtomicUsize::new(0);
    struct SecondAbortHook;
    impl FacilitatorHooks for SecondAbortHook {
        fn before_verify<'a>(&'a self, _: &'a VerifyContext) -> BoxFuture<'a, HookDecision> {
            Box::pin(async {
                SECOND_HOOK_CALLS.fetch_add(1, Ordering::Relaxed);
                HookDecision::Abort {
                    reason: "second".into(),
                    message: String::new(),
                }
            })
        }
    }

    fn dummy_request() -> proto::VerifyRequest {
        serde_json::json!({}).into()
    }

    fn dummy_settle() -> proto::SettleRequest {
        serde_json::json!({}).into()
    }

    #[tokio::test]
    async fn verify_no_hooks_passes_through() {
        let hooked = HookedFacilitator::new(MockFacilitator::ok());
        assert_eq!(hooked.hook_count(), 0);
        let resp = hooked.verify(dummy_request()).await.unwrap();
        assert!(resp.is_valid());
    }

    #[tokio::test]
    async fn verify_before_hook_aborts() {
        let hooked = HookedFacilitator::new(MockFacilitator::ok()).with_hook(AbortVerifyHook);
        let err = hooked.verify(dummy_request()).await.unwrap_err();
        assert!(matches!(err, FacilitatorError::Aborted { reason, .. } if reason == "blocked"));
    }

    #[tokio::test]
    async fn verify_failure_hook_recovers() {
        let hooked =
            HookedFacilitator::new(MockFacilitator::failing()).with_hook(RecoverVerifyHook);
        let resp = hooked.verify(dummy_request()).await.unwrap();
        assert!(resp.is_valid());
    }

    #[tokio::test]
    async fn verify_failure_hook_propagates_by_default() {
        let hooked = HookedFacilitator::new(MockFacilitator::failing()).with_hook(NoopHook);
        assert!(hooked.verify(dummy_request()).await.is_err());
    }

    #[tokio::test]
    async fn settle_before_hook_aborts() {
        let hooked = HookedFacilitator::new(MockFacilitator::ok()).with_hook(AbortSettleHook);
        let err = hooked.settle(dummy_settle()).await.unwrap_err();
        assert!(matches!(err, FacilitatorError::Aborted { reason, .. } if reason == "blocked"));
    }

    #[tokio::test]
    async fn settle_success_passes_through() {
        let hooked = HookedFacilitator::new(MockFacilitator::ok());
        let resp = hooked.settle(dummy_settle()).await.unwrap();
        assert!(resp.is_success());
    }

    #[tokio::test]
    async fn settle_failure_hook_recovers() {
        let hooked =
            HookedFacilitator::new(MockFacilitator::failing()).with_hook(RecoverSettleHook);
        let resp = hooked.settle(dummy_settle()).await.unwrap();
        assert!(resp.is_success());
    }

    #[tokio::test]
    async fn first_abort_hook_wins_remaining_skipped() {
        SECOND_HOOK_CALLS.store(0, Ordering::Relaxed);

        let hooked = HookedFacilitator::new(MockFacilitator::ok())
            .with_hook(AbortVerifyHook)
            .with_hook(SecondAbortHook);

        let err = hooked.verify(dummy_request()).await.unwrap_err();
        assert!(matches!(err, FacilitatorError::Aborted { reason, .. } if reason == "blocked"));
        assert_eq!(
            SECOND_HOOK_CALLS.load(Ordering::Relaxed),
            0,
            "second hook must not run after first aborts"
        );
    }

    #[tokio::test]
    async fn add_hook_dynamic() {
        let mut hooked = HookedFacilitator::new(MockFacilitator::ok());
        assert_eq!(hooked.hook_count(), 0);
        hooked.add_hook(NoopHook);
        assert_eq!(hooked.hook_count(), 1);
        let resp = hooked.verify(dummy_request()).await.unwrap();
        assert!(resp.is_valid());
    }

    #[tokio::test]
    async fn supported_delegates_to_inner() {
        let hooked = HookedFacilitator::new(MockFacilitator::ok());
        let resp = hooked.supported().await.unwrap();
        assert!(resp.kinds.is_empty());
        assert!(resp.signers.is_empty());
    }
}
