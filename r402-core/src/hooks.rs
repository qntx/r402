//! Lifecycle hooks for facilitator verify / settle operations.
//!
//! A [`HookedFacilitator`] wraps any [`Facilitator`] and runs registered
//! [`FacilitatorHooks`] at three lifecycle points:
//!
//! 1. **Before** — inspect or abort the operation.
//! 2. **After** — observe a successful result.
//! 3. **On failure** — observe or recover from an error.
//!
//! Hook methods use AFIT for zero-cost static dispatch when the hook type is
//! statically known. For heterogeneous lists (e.g. a registry collecting
//! multiple hook implementations) use the [`DynFacilitatorHooks`] erasure.

use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::pin::Pin;

use crate::error::FacilitatorError;
use crate::facilitator::{BoxFuture, Facilitator};
use crate::wire::{
    SettleRequest, SettleResponse, SupportedResponse, VerifyRequest, VerifyResponse,
};

/// Decision returned by "before" hooks to control whether the operation
/// proceeds.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum HookDecision {
    /// Continue execution normally.
    Continue,
    /// Abort with a structured reason + message.
    Abort {
        /// Machine-readable reason for aborting.
        reason: String,
        /// Human-readable description.
        message: String,
    },
}

/// Outcome returned by "on failure" hooks, indicating whether recovery
/// happened.
#[derive(Debug)]
#[non_exhaustive]
pub enum FailureRecovery<T> {
    /// No recovery — propagate the original error.
    Propagate,
    /// The hook produced a substitute success result.
    Recovered(T),
}

/// Context passed to verify-related hooks.
#[derive(Clone)]
pub struct VerifyContext {
    /// The incoming verify request.
    pub request: VerifyRequest,
}

impl Debug for VerifyContext {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerifyContext").finish_non_exhaustive()
    }
}

/// Context passed to settle-related hooks.
#[derive(Clone)]
pub struct SettleContext {
    /// The incoming settle request.
    pub request: SettleRequest,
}

impl Debug for SettleContext {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SettleContext").finish_non_exhaustive()
    }
}

/// Lifecycle hooks for facilitator verify and settle operations.
///
/// All methods default to a no-op — implementers override only what they
/// need. The trait uses AFIT for static dispatch; see
/// [`DynFacilitatorHooks`] for the object-safe erasure.
pub trait FacilitatorHooks: Send + Sync {
    /// Runs before every `verify` call.
    fn before_verify<'a>(
        &'a self,
        _ctx: &'a VerifyContext,
    ) -> impl Future<Output = HookDecision> + Send + 'a {
        async { HookDecision::Continue }
    }

    /// Runs after every successful `verify` call.
    fn after_verify<'a>(
        &'a self,
        _ctx: &'a VerifyContext,
        _response: &'a VerifyResponse,
    ) -> impl Future<Output = ()> + Send + 'a {
        async {}
    }

    /// Runs when a `verify` call returns an error.
    fn on_verify_failure<'a>(
        &'a self,
        _ctx: &'a VerifyContext,
        _error: &'a FacilitatorError,
    ) -> impl Future<Output = FailureRecovery<VerifyResponse>> + Send + 'a {
        async { FailureRecovery::Propagate }
    }

    /// Runs before every `settle` call.
    fn before_settle<'a>(
        &'a self,
        _ctx: &'a SettleContext,
    ) -> impl Future<Output = HookDecision> + Send + 'a {
        async { HookDecision::Continue }
    }

    /// Runs after every successful `settle` call.
    fn after_settle<'a>(
        &'a self,
        _ctx: &'a SettleContext,
        _response: &'a SettleResponse,
    ) -> impl Future<Output = ()> + Send + 'a {
        async {}
    }

    /// Runs when a `settle` call returns an error.
    fn on_settle_failure<'a>(
        &'a self,
        _ctx: &'a SettleContext,
        _error: &'a FacilitatorError,
    ) -> impl Future<Output = FailureRecovery<SettleResponse>> + Send + 'a {
        async { FailureRecovery::Propagate }
    }
}

/// Object-safe erasure of [`FacilitatorHooks`].
pub trait DynFacilitatorHooks: Send + Sync {
    /// See [`FacilitatorHooks::before_verify`].
    fn before_verify<'a>(
        &'a self,
        ctx: &'a VerifyContext,
    ) -> Pin<Box<dyn Future<Output = HookDecision> + Send + 'a>>;

    /// See [`FacilitatorHooks::after_verify`].
    fn after_verify<'a>(
        &'a self,
        ctx: &'a VerifyContext,
        response: &'a VerifyResponse,
    ) -> BoxFuture<'a, ()>;

    /// See [`FacilitatorHooks::on_verify_failure`].
    fn on_verify_failure<'a>(
        &'a self,
        ctx: &'a VerifyContext,
        error: &'a FacilitatorError,
    ) -> BoxFuture<'a, FailureRecovery<VerifyResponse>>;

    /// See [`FacilitatorHooks::before_settle`].
    fn before_settle<'a>(&'a self, ctx: &'a SettleContext) -> BoxFuture<'a, HookDecision>;

    /// See [`FacilitatorHooks::after_settle`].
    fn after_settle<'a>(
        &'a self,
        ctx: &'a SettleContext,
        response: &'a SettleResponse,
    ) -> BoxFuture<'a, ()>;

    /// See [`FacilitatorHooks::on_settle_failure`].
    fn on_settle_failure<'a>(
        &'a self,
        ctx: &'a SettleContext,
        error: &'a FacilitatorError,
    ) -> BoxFuture<'a, FailureRecovery<SettleResponse>>;
}

impl<T: FacilitatorHooks + ?Sized> DynFacilitatorHooks for T {
    fn before_verify<'a>(&'a self, ctx: &'a VerifyContext) -> BoxFuture<'a, HookDecision> {
        Box::pin(<Self as FacilitatorHooks>::before_verify(self, ctx))
    }

    fn after_verify<'a>(
        &'a self,
        ctx: &'a VerifyContext,
        response: &'a VerifyResponse,
    ) -> BoxFuture<'a, ()> {
        Box::pin(<Self as FacilitatorHooks>::after_verify(
            self, ctx, response,
        ))
    }

    fn on_verify_failure<'a>(
        &'a self,
        ctx: &'a VerifyContext,
        error: &'a FacilitatorError,
    ) -> BoxFuture<'a, FailureRecovery<VerifyResponse>> {
        Box::pin(<Self as FacilitatorHooks>::on_verify_failure(
            self, ctx, error,
        ))
    }

    fn before_settle<'a>(&'a self, ctx: &'a SettleContext) -> BoxFuture<'a, HookDecision> {
        Box::pin(<Self as FacilitatorHooks>::before_settle(self, ctx))
    }

    fn after_settle<'a>(
        &'a self,
        ctx: &'a SettleContext,
        response: &'a SettleResponse,
    ) -> BoxFuture<'a, ()> {
        Box::pin(<Self as FacilitatorHooks>::after_settle(
            self, ctx, response,
        ))
    }

    fn on_settle_failure<'a>(
        &'a self,
        ctx: &'a SettleContext,
        error: &'a FacilitatorError,
    ) -> BoxFuture<'a, FailureRecovery<SettleResponse>> {
        Box::pin(<Self as FacilitatorHooks>::on_settle_failure(
            self, ctx, error,
        ))
    }
}

/// Facilitator decorator that runs registered hooks around verify and settle.
///
/// Hook invocation order:
///
/// - **Before** hooks run in registration order; first `Abort` wins.
/// - **After** hooks run in registration order; errors are silently dropped.
/// - **On-failure** hooks run in registration order; first `Recovered` wins.
pub struct HookedFacilitator<F> {
    inner: F,
    hooks: Vec<Box<dyn DynFacilitatorHooks>>,
}

impl<F: Debug> Debug for HookedFacilitator<F> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("HookedFacilitator")
            .field("inner", &self.inner)
            .field("hooks", &format_args!("[{} hooks]", self.hooks.len()))
            .finish()
    }
}

impl<F> HookedFacilitator<F> {
    /// Wraps `inner` with an empty hook list.
    pub const fn new(inner: F) -> Self {
        Self {
            inner,
            hooks: Vec::new(),
        }
    }

    /// Registers a hook. Returns `self` so builder-style chaining works.
    #[must_use]
    pub fn with_hook(mut self, hook: impl FacilitatorHooks + 'static) -> Self {
        self.hooks.push(Box::new(hook));
        self
    }

    /// Registers a hook after construction.
    pub fn add_hook(&mut self, hook: impl FacilitatorHooks + 'static) {
        self.hooks.push(Box::new(hook));
    }

    /// Number of registered hooks.
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

impl<F: Sync> HookedFacilitator<F> {
    async fn run_before_verify(&self, ctx: &VerifyContext) -> Result<(), FacilitatorError> {
        for hook in &self.hooks {
            if let HookDecision::Abort { reason, message } = hook.before_verify(ctx).await {
                return Err(FacilitatorError::Aborted { reason, message });
            }
        }
        Ok(())
    }

    async fn run_after_verify(&self, ctx: &VerifyContext, response: &VerifyResponse) {
        for hook in &self.hooks {
            hook.after_verify(ctx, response).await;
        }
    }

    async fn run_on_verify_failure(
        &self,
        ctx: &VerifyContext,
        error: &FacilitatorError,
    ) -> Option<VerifyResponse> {
        for hook in &self.hooks {
            if let FailureRecovery::Recovered(response) = hook.on_verify_failure(ctx, error).await {
                return Some(response);
            }
        }
        None
    }

    async fn run_before_settle(&self, ctx: &SettleContext) -> Result<(), FacilitatorError> {
        for hook in &self.hooks {
            if let HookDecision::Abort { reason, message } = hook.before_settle(ctx).await {
                return Err(FacilitatorError::Aborted { reason, message });
            }
        }
        Ok(())
    }

    async fn run_after_settle(&self, ctx: &SettleContext, response: &SettleResponse) {
        for hook in &self.hooks {
            hook.after_settle(ctx, response).await;
        }
    }

    async fn run_on_settle_failure(
        &self,
        ctx: &SettleContext,
        error: &FacilitatorError,
    ) -> Option<SettleResponse> {
        for hook in &self.hooks {
            if let FailureRecovery::Recovered(response) = hook.on_settle_failure(ctx, error).await {
                return Some(response);
            }
        }
        None
    }
}

impl<F: Facilitator + Sync> Facilitator for HookedFacilitator<F> {
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        let ctx = VerifyContext {
            request: request.clone(),
        };
        self.run_before_verify(&ctx).await?;
        match self.inner.verify(request).await {
            Ok(response) => {
                self.run_after_verify(&ctx, &response).await;
                Ok(response)
            }
            Err(error) => {
                let recovered = self.run_on_verify_failure(&ctx, &error).await;
                recovered.ok_or(error)
            }
        }
    }

    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        let ctx = SettleContext {
            request: request.clone(),
        };
        self.run_before_settle(&ctx).await?;
        match self.inner.settle(request).await {
            Ok(response) => {
                self.run_after_settle(&ctx, &response).await;
                Ok(response)
            }
            Err(error) => {
                let recovered = self.run_on_settle_failure(&ctx, &error).await;
                recovered.ok_or(error)
            }
        }
    }

    async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
        self.inner.supported().await
    }
}

#[cfg(test)]
#[allow(
    clippy::excessive_nesting,
    reason = "mock facilitator impls inherently nest async fn bodies inside impl-in-fn"
)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::error_reason::ErrorReason;
    use crate::wire::Extensions;

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
    }

    impl Facilitator for MockFacilitator {
        async fn verify(
            &self,
            _request: VerifyRequest,
        ) -> Result<VerifyResponse, FacilitatorError> {
            if self.fail {
                Err(FacilitatorError::Onchain("mock".into()))
            } else {
                Ok(VerifyResponse::valid("0xPAYER"))
            }
        }

        async fn settle(
            &self,
            _request: SettleRequest,
        ) -> Result<SettleResponse, FacilitatorError> {
            if self.fail {
                Err(FacilitatorError::Onchain("mock".into()))
            } else {
                Ok(SettleResponse::Success {
                    payer: "0xPAYER".into(),
                    transaction: "0xTX".into(),
                    network: "eip155:1".into(),
                    amount: None,
                    extensions: Extensions::new(),
                })
            }
        }

        async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
            Ok(SupportedResponse::default())
        }
    }

    struct AbortVerifyHook;
    impl FacilitatorHooks for AbortVerifyHook {
        async fn before_verify(&self, _: &VerifyContext) -> HookDecision {
            HookDecision::Abort {
                reason: "blocked".into(),
                message: "test".into(),
            }
        }
    }

    struct AbortSettleHook;
    impl FacilitatorHooks for AbortSettleHook {
        async fn before_settle(&self, _: &SettleContext) -> HookDecision {
            HookDecision::Abort {
                reason: "blocked".into(),
                message: "test".into(),
            }
        }
    }

    struct RecoverVerifyHook;
    impl FacilitatorHooks for RecoverVerifyHook {
        async fn on_verify_failure(
            &self,
            _: &VerifyContext,
            _: &FacilitatorError,
        ) -> FailureRecovery<VerifyResponse> {
            FailureRecovery::Recovered(VerifyResponse::valid("0xREC"))
        }
    }

    struct RecoverSettleHook;
    impl FacilitatorHooks for RecoverSettleHook {
        async fn on_settle_failure(
            &self,
            _: &SettleContext,
            _: &FacilitatorError,
        ) -> FailureRecovery<SettleResponse> {
            FailureRecovery::Recovered(SettleResponse::Success {
                payer: "0xREC".into(),
                transaction: "0xREC_TX".into(),
                network: "eip155:1".into(),
                amount: None,
                extensions: Extensions::new(),
            })
        }
    }

    struct NoopHook;
    impl FacilitatorHooks for NoopHook {}

    struct SecondAbortHook(&'static AtomicUsize);
    impl FacilitatorHooks for SecondAbortHook {
        async fn before_verify(&self, _: &VerifyContext) -> HookDecision {
            let _ = self.0.fetch_add(1, Ordering::Relaxed);
            HookDecision::Abort {
                reason: "second".into(),
                message: String::new(),
            }
        }
    }

    fn dummy_verify() -> VerifyRequest {
        serde_json::json!({}).into()
    }
    fn dummy_settle() -> SettleRequest {
        serde_json::json!({}).into()
    }

    #[tokio::test]
    async fn verify_no_hooks_passes_through() {
        let hooked = HookedFacilitator::new(MockFacilitator::ok());
        assert_eq!(hooked.hook_count(), 0);
        let response = hooked.verify(dummy_verify()).await.unwrap();
        assert!(response.is_valid());
    }

    #[tokio::test]
    async fn verify_before_hook_aborts() {
        let hooked = HookedFacilitator::new(MockFacilitator::ok()).with_hook(AbortVerifyHook);
        let err = hooked.verify(dummy_verify()).await.unwrap_err();
        assert!(matches!(err, FacilitatorError::Aborted { reason, .. } if reason == "blocked"));
    }

    #[tokio::test]
    async fn verify_failure_hook_recovers() {
        let hooked =
            HookedFacilitator::new(MockFacilitator::failing()).with_hook(RecoverVerifyHook);
        assert!(hooked.verify(dummy_verify()).await.unwrap().is_valid());
    }

    #[tokio::test]
    async fn verify_failure_hook_propagates_by_default() {
        let hooked = HookedFacilitator::new(MockFacilitator::failing()).with_hook(NoopHook);
        assert!(hooked.verify(dummy_verify()).await.is_err());
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
        assert!(hooked.settle(dummy_settle()).await.unwrap().is_success());
    }

    #[tokio::test]
    async fn settle_failure_hook_recovers() {
        let hooked =
            HookedFacilitator::new(MockFacilitator::failing()).with_hook(RecoverSettleHook);
        assert!(hooked.settle(dummy_settle()).await.unwrap().is_success());
    }

    #[tokio::test]
    async fn first_abort_wins_remaining_skipped() {
        static SECOND_CALLS: AtomicUsize = AtomicUsize::new(0);
        SECOND_CALLS.store(0, Ordering::Relaxed);
        let hooked = HookedFacilitator::new(MockFacilitator::ok())
            .with_hook(AbortVerifyHook)
            .with_hook(SecondAbortHook(&SECOND_CALLS));
        let err = hooked.verify(dummy_verify()).await.unwrap_err();
        assert!(matches!(err, FacilitatorError::Aborted { reason, .. } if reason == "blocked"));
        assert_eq!(SECOND_CALLS.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn add_hook_dynamic() {
        let mut hooked = HookedFacilitator::new(MockFacilitator::ok());
        assert_eq!(hooked.hook_count(), 0);
        hooked.add_hook(NoopHook);
        assert_eq!(hooked.hook_count(), 1);
        assert!(hooked.verify(dummy_verify()).await.unwrap().is_valid());
    }

    #[tokio::test]
    async fn supported_delegates_to_inner() {
        let hooked = HookedFacilitator::new(MockFacilitator::ok());
        let response = hooked.supported().await.unwrap();
        assert!(response.kinds.is_empty());
        assert!(response.signers.is_empty());
    }

    #[tokio::test]
    async fn verify_invalid_response_with_reason_round_trip() {
        struct Invalid;
        impl Facilitator for Invalid {
            async fn verify(&self, _r: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
                Ok(VerifyResponse::invalid(None, ErrorReason::InvalidPayload))
            }
            async fn settle(&self, _r: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
                unreachable!()
            }
            async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
                Ok(SupportedResponse::default())
            }
        }
        let hooked = HookedFacilitator::new(Invalid);
        let response = hooked.verify(dummy_verify()).await.unwrap();
        match response {
            VerifyResponse::Invalid { reason, .. } => {
                assert_eq!(reason, ErrorReason::InvalidPayload);
            }
            VerifyResponse::Valid { .. } => panic!("expected invalid"),
        }
    }
}
