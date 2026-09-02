//! Unified asynchronous interface for x402 payment facilitators.
//!
//! The core [`Facilitator`] trait uses Rust's native AFIT (async fn in traits)
//! for **zero-cost static dispatch**. For use cases that need a dyn-compatible
//! trait (heterogeneous registries, hook pipelines), the
//! [`DynFacilitator`] trait erases the concrete future type via
//! `Pin<Box<dyn Future>>`. A blanket implementation turns every
//! `Facilitator` into a `DynFacilitator` automatically.
//!
//! [`HookedFacilitator`] wraps any [`Facilitator`] and runs registered
//! [`FacilitatorHooks`] around verify / settle.

use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::error::FacilitatorError;
use crate::wire::{
    SettleRequest, SettleResponse, SupportedResponse, VerifyRequest, VerifyResponse,
};

/// Boxed future alias used by [`DynFacilitator`] and legacy code.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Unified asynchronous interface for x402 facilitators.
///
/// This trait is **not** dyn-compatible because it uses `async fn` in traits.
/// Callers that need a trait object should use [`DynFacilitator`] instead
/// (the blanket impl below makes every `Facilitator` automatically satisfy it).
pub trait Facilitator: Send + Sync {
    /// Verifies a payment payload against its requirements.
    fn verify(
        &self,
        request: VerifyRequest,
    ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send;

    /// Settles a previously verified payment on-chain.
    fn settle(
        &self,
        request: SettleRequest,
    ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send;

    /// Returns the payment kinds supported by this facilitator.
    fn supported(&self)
    -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send;
}

/// Object-safe erasure of [`Facilitator`], suitable for storing as
/// `Box<dyn DynFacilitator>` or `Arc<dyn DynFacilitator>`.
pub trait DynFacilitator: Send + Sync {
    /// See [`Facilitator::verify`].
    fn verify(
        &self,
        request: VerifyRequest,
    ) -> BoxFuture<'_, Result<VerifyResponse, FacilitatorError>>;

    /// See [`Facilitator::settle`].
    fn settle(
        &self,
        request: SettleRequest,
    ) -> BoxFuture<'_, Result<SettleResponse, FacilitatorError>>;

    /// See [`Facilitator::supported`].
    fn supported(&self) -> BoxFuture<'_, Result<SupportedResponse, FacilitatorError>>;
}

impl<T> DynFacilitator for T
where
    T: Facilitator + ?Sized,
{
    fn verify(
        &self,
        request: VerifyRequest,
    ) -> BoxFuture<'_, Result<VerifyResponse, FacilitatorError>> {
        Box::pin(<Self as Facilitator>::verify(self, request))
    }

    fn settle(
        &self,
        request: SettleRequest,
    ) -> BoxFuture<'_, Result<SettleResponse, FacilitatorError>> {
        Box::pin(<Self as Facilitator>::settle(self, request))
    }

    fn supported(&self) -> BoxFuture<'_, Result<SupportedResponse, FacilitatorError>> {
        Box::pin(<Self as Facilitator>::supported(self))
    }
}

impl<T: Facilitator> Facilitator for Arc<T> {
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        self.as_ref().verify(request).await
    }

    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        self.as_ref().settle(request).await
    }

    async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
        self.as_ref().supported().await
    }
}

impl<T: Facilitator> Facilitator for Box<T> {
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        self.as_ref().verify(request).await
    }

    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        self.as_ref().settle(request).await
    }

    async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
        self.as_ref().supported().await
    }
}

impl Facilitator for Box<dyn DynFacilitator> {
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        DynFacilitator::verify(self.as_ref(), request).await
    }

    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        DynFacilitator::settle(self.as_ref(), request).await
    }

    async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
        DynFacilitator::supported(self.as_ref()).await
    }
}

impl Facilitator for Arc<dyn DynFacilitator> {
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        DynFacilitator::verify(self.as_ref(), request).await
    }

    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        DynFacilitator::settle(self.as_ref(), request).await
    }

    async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
        DynFacilitator::supported(self.as_ref()).await
    }
}

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
#[non_exhaustive]
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
#[non_exhaustive]
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
///
/// With the `metrics` feature, each `verify` / `settle` records the
/// `r402_facilitator_*` counters and duration histograms.
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

impl<F: Facilitator + Sync> HookedFacilitator<F> {
    async fn verify_hooked(
        &self,
        request: VerifyRequest,
    ) -> Result<VerifyResponse, FacilitatorError> {
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

    async fn settle_hooked(
        &self,
        request: SettleRequest,
    ) -> Result<SettleResponse, FacilitatorError> {
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
}

impl<F: Facilitator + Sync> Facilitator for HookedFacilitator<F> {
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        #[cfg(feature = "metrics")]
        let started = std::time::Instant::now();
        let result = self.verify_hooked(request).await;
        #[cfg(feature = "metrics")]
        record_verify_metrics(&result, started.elapsed());
        result
    }

    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        #[cfg(feature = "metrics")]
        let started = std::time::Instant::now();
        let result = self.settle_hooked(request).await;
        #[cfg(feature = "metrics")]
        record_settle_metrics(&result, started.elapsed());
        result
    }

    async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
        self.inner.supported().await
    }
}

#[cfg(feature = "metrics")]
fn record_verify_metrics(
    result: &Result<VerifyResponse, FacilitatorError>,
    elapsed: std::time::Duration,
) {
    let label = match result {
        Ok(VerifyResponse::Valid { .. }) => "valid",
        Ok(VerifyResponse::Invalid { .. }) => "invalid",
        Err(_) => "error",
    };
    ::metrics::counter!(
        crate::metrics::FACILITATOR_VERIFY_TOTAL,
        "result" => label,
    )
    .increment(1);
    ::metrics::histogram!(crate::metrics::FACILITATOR_VERIFY_DURATION_SECONDS).record(elapsed);
}

#[cfg(feature = "metrics")]
fn record_settle_metrics(
    result: &Result<SettleResponse, FacilitatorError>,
    elapsed: std::time::Duration,
) {
    let label = match result {
        Ok(SettleResponse::Success { .. }) => "success",
        Ok(SettleResponse::Failure { .. }) => "failure",
        Err(_) => "error",
    };
    ::metrics::counter!(
        crate::metrics::FACILITATOR_SETTLE_TOTAL,
        "result" => label,
    )
    .increment(1);
    ::metrics::histogram!(crate::metrics::FACILITATOR_SETTLE_DURATION_SECONDS).record(elapsed);
}

#[cfg(test)]
#[allow(
    clippy::excessive_nesting,
    reason = "mock facilitator impls inherently nest async fn bodies inside impl-in-fn"
)]
mod tests {
    use std::future::Future;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{
        Facilitator, FacilitatorError, FacilitatorHooks, FailureRecovery, HookDecision,
        HookedFacilitator, SettleContext, SettleRequest, SettleResponse, SupportedResponse,
        VerifyContext, VerifyRequest, VerifyResponse,
    };
    use crate::error::ErrorReason;
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
        fn verify(
            &self,
            _request: VerifyRequest,
        ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
            let result = if self.fail {
                Err(FacilitatorError::Onchain("mock".into()))
            } else {
                Ok(VerifyResponse::valid("0xPAYER"))
            };
            std::future::ready(result)
        }

        fn settle(
            &self,
            _request: SettleRequest,
        ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
            let result = if self.fail {
                Err(FacilitatorError::Onchain("mock".into()))
            } else {
                Ok(SettleResponse::Success {
                    payer: "0xPAYER".into(),
                    transaction: "0xTX".into(),
                    network: "eip155:1".into(),
                    amount: None,
                    extensions: Extensions::new(),
                })
            };
            std::future::ready(result)
        }

        fn supported(
            &self,
        ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(SupportedResponse::default()))
        }
    }

    struct AbortVerifyHook;
    impl FacilitatorHooks for AbortVerifyHook {
        fn before_verify<'a>(
            &'a self,
            _: &VerifyContext,
        ) -> impl Future<Output = HookDecision> + Send + 'a {
            std::future::ready(HookDecision::Abort {
                reason: "blocked".into(),
                message: "test".into(),
            })
        }
    }

    struct AbortSettleHook;
    impl FacilitatorHooks for AbortSettleHook {
        fn before_settle<'a>(
            &'a self,
            _: &SettleContext,
        ) -> impl Future<Output = HookDecision> + Send + 'a {
            std::future::ready(HookDecision::Abort {
                reason: "blocked".into(),
                message: "test".into(),
            })
        }
    }

    struct RecoverVerifyHook;
    impl FacilitatorHooks for RecoverVerifyHook {
        fn on_verify_failure<'a>(
            &'a self,
            _: &VerifyContext,
            _: &FacilitatorError,
        ) -> impl Future<Output = FailureRecovery<VerifyResponse>> + Send + 'a {
            std::future::ready(FailureRecovery::Recovered(VerifyResponse::valid("0xREC")))
        }
    }

    struct RecoverSettleHook;
    impl FacilitatorHooks for RecoverSettleHook {
        fn on_settle_failure<'a>(
            &'a self,
            _: &SettleContext,
            _: &FacilitatorError,
        ) -> impl Future<Output = FailureRecovery<SettleResponse>> + Send + 'a {
            std::future::ready(FailureRecovery::Recovered(SettleResponse::Success {
                payer: "0xREC".into(),
                transaction: "0xREC_TX".into(),
                network: "eip155:1".into(),
                amount: None,
                extensions: Extensions::new(),
            }))
        }
    }

    struct NoopHook;
    impl FacilitatorHooks for NoopHook {}

    struct SecondAbortHook(&'static AtomicUsize);
    impl FacilitatorHooks for SecondAbortHook {
        fn before_verify<'a>(
            &'a self,
            _: &VerifyContext,
        ) -> impl Future<Output = HookDecision> + Send + 'a {
            let _ = self.0.fetch_add(1, Ordering::Relaxed);
            std::future::ready(HookDecision::Abort {
                reason: "second".into(),
                message: String::new(),
            })
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
            fn verify(
                &self,
                _r: VerifyRequest,
            ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
                std::future::ready(Ok(VerifyResponse::invalid(
                    None,
                    ErrorReason::InvalidPayload,
                )))
            }
            fn settle(
                &self,
                _r: SettleRequest,
            ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
                std::future::ready(Err(FacilitatorError::Onchain("unreachable".into())))
            }
            fn supported(
                &self,
            ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send
            {
                std::future::ready(Ok(SupportedResponse::default()))
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

    #[cfg(feature = "metrics")]
    mod metrics_recording {
        use std::future::Future;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU64, Ordering};

        use metrics::{
            Counter, CounterFn, Gauge, Histogram, HistogramFn, Key, KeyName, Metadata, Recorder,
            SharedString, Unit,
        };

        use super::{
            AbortSettleHook, AbortVerifyHook, Facilitator, FacilitatorError, HookedFacilitator,
            MockFacilitator, RecoverVerifyHook, SettleRequest, SettleResponse, SupportedResponse,
            VerifyRequest, VerifyResponse, dummy_settle, dummy_verify,
        };
        use crate::error::ErrorReason;
        use crate::metrics::{
            FACILITATOR_SETTLE_DURATION_SECONDS, FACILITATOR_SETTLE_TOTAL,
            FACILITATOR_VERIFY_DURATION_SECONDS, FACILITATOR_VERIFY_TOTAL,
        };
        use crate::wire::Extensions;

        struct Count(AtomicU64);

        impl Count {
            fn arc() -> Arc<Self> {
                Arc::new(Self(AtomicU64::new(0)))
            }

            fn get(handle: &Arc<Self>) -> u64 {
                handle.0.load(Ordering::Relaxed)
            }
        }

        impl CounterFn for Count {
            fn increment(&self, value: u64) {
                self.0.fetch_add(value, Ordering::Relaxed);
            }

            fn absolute(&self, value: u64) {
                self.0.store(value, Ordering::Relaxed);
            }
        }

        impl HistogramFn for Count {
            fn record(&self, _value: f64) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }

        struct Capture {
            verify_valid: Arc<Count>,
            verify_invalid: Arc<Count>,
            verify_error: Arc<Count>,
            verify_duration: Arc<Count>,
            settle_success: Arc<Count>,
            settle_failure: Arc<Count>,
            settle_error: Arc<Count>,
            settle_duration: Arc<Count>,
        }

        impl Capture {
            fn new() -> Self {
                Self {
                    verify_valid: Count::arc(),
                    verify_invalid: Count::arc(),
                    verify_error: Count::arc(),
                    verify_duration: Count::arc(),
                    settle_success: Count::arc(),
                    settle_failure: Count::arc(),
                    settle_error: Count::arc(),
                    settle_duration: Count::arc(),
                }
            }

            fn counter_handle(&self, name: &str, result: &str) -> Option<Arc<Count>> {
                Some(Arc::clone(match (name, result) {
                    (FACILITATOR_VERIFY_TOTAL, "valid") => &self.verify_valid,
                    (FACILITATOR_VERIFY_TOTAL, "invalid") => &self.verify_invalid,
                    (FACILITATOR_VERIFY_TOTAL, "error") => &self.verify_error,
                    (FACILITATOR_SETTLE_TOTAL, "success") => &self.settle_success,
                    (FACILITATOR_SETTLE_TOTAL, "failure") => &self.settle_failure,
                    (FACILITATOR_SETTLE_TOTAL, "error") => &self.settle_error,
                    _ => return None,
                }))
            }

            fn histogram_handle(&self, name: &str) -> Option<Arc<Count>> {
                Some(Arc::clone(match name {
                    FACILITATOR_VERIFY_DURATION_SECONDS => &self.verify_duration,
                    FACILITATOR_SETTLE_DURATION_SECONDS => &self.settle_duration,
                    _ => return None,
                }))
            }
        }

        impl Recorder for Capture {
            fn describe_counter(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
            fn describe_gauge(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
            fn describe_histogram(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}

            fn register_counter(&self, key: &Key, _: &Metadata<'_>) -> Counter {
                let result = key
                    .labels()
                    .find(|label| label.key() == "result")
                    .map_or("", |label| label.value());
                self.counter_handle(key.name(), result)
                    .map_or_else(Counter::noop, Counter::from_arc)
            }

            fn register_gauge(&self, _: &Key, _: &Metadata<'_>) -> Gauge {
                Gauge::noop()
            }

            fn register_histogram(&self, key: &Key, _: &Metadata<'_>) -> Histogram {
                self.histogram_handle(key.name())
                    .map_or_else(Histogram::noop, Histogram::from_arc)
            }
        }

        struct InvalidVerify;
        impl Facilitator for InvalidVerify {
            fn verify(
                &self,
                _request: VerifyRequest,
            ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
                std::future::ready(Ok(VerifyResponse::invalid(
                    None,
                    ErrorReason::InvalidPayload,
                )))
            }
            fn settle(
                &self,
                _request: SettleRequest,
            ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
                std::future::ready(Err(FacilitatorError::Onchain("unreachable".into())))
            }
            fn supported(
                &self,
            ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send
            {
                std::future::ready(Ok(SupportedResponse::default()))
            }
        }

        struct FailedSettle;
        impl Facilitator for FailedSettle {
            fn verify(
                &self,
                _request: VerifyRequest,
            ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
                std::future::ready(Err(FacilitatorError::Onchain("unreachable".into())))
            }
            fn settle(
                &self,
                _request: SettleRequest,
            ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
                std::future::ready(Ok(SettleResponse::Failure {
                    reason: ErrorReason::UnexpectedSettleError,
                    message: None,
                    payer: None,
                    network: "eip155:1".into(),
                    extensions: Extensions::new(),
                }))
            }
            fn supported(
                &self,
            ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send
            {
                std::future::ready(Ok(SupportedResponse::default()))
            }
        }

        #[tokio::test]
        async fn hooked_verify_records_result_and_duration() {
            let capture = Capture::new();
            let _guard = metrics::set_default_local_recorder(&capture);

            let ok = HookedFacilitator::new(MockFacilitator::ok());
            assert!(
                ok.verify(dummy_verify()).await.unwrap().is_valid(),
                "ok verify"
            );

            let recovered =
                HookedFacilitator::new(MockFacilitator::failing()).with_hook(RecoverVerifyHook);
            assert!(
                recovered.verify(dummy_verify()).await.unwrap().is_valid(),
                "recovered verify"
            );

            let invalid = HookedFacilitator::new(InvalidVerify);
            assert!(
                !invalid.verify(dummy_verify()).await.unwrap().is_valid(),
                "invalid verify"
            );

            let failing = HookedFacilitator::new(MockFacilitator::failing());
            assert!(
                failing.verify(dummy_verify()).await.is_err(),
                "failing verify"
            );

            let aborted = HookedFacilitator::new(MockFacilitator::ok()).with_hook(AbortVerifyHook);
            assert!(
                aborted.verify(dummy_verify()).await.is_err(),
                "aborted verify"
            );

            assert_eq!(Count::get(&capture.verify_valid), 2, "valid+recovered");
            assert_eq!(Count::get(&capture.verify_invalid), 1, "invalid");
            assert_eq!(Count::get(&capture.verify_error), 2, "error+abort");
            assert_eq!(Count::get(&capture.verify_duration), 5, "verify samples");
        }

        #[tokio::test]
        async fn hooked_settle_records_result_and_duration() {
            let capture = Capture::new();
            let _guard = metrics::set_default_local_recorder(&capture);

            let ok = HookedFacilitator::new(MockFacilitator::ok());
            assert!(
                ok.settle(dummy_settle()).await.unwrap().is_success(),
                "ok settle"
            );

            let failed = HookedFacilitator::new(FailedSettle);
            assert!(
                !failed.settle(dummy_settle()).await.unwrap().is_success(),
                "failed settle"
            );

            let failing = HookedFacilitator::new(MockFacilitator::failing());
            assert!(
                failing.settle(dummy_settle()).await.is_err(),
                "failing settle"
            );

            let aborted = HookedFacilitator::new(MockFacilitator::ok()).with_hook(AbortSettleHook);
            assert!(
                aborted.settle(dummy_settle()).await.is_err(),
                "aborted settle"
            );

            assert_eq!(Count::get(&capture.settle_success), 1, "success");
            assert_eq!(Count::get(&capture.settle_failure), 1, "failure");
            assert_eq!(Count::get(&capture.settle_error), 2, "error+abort");
            assert_eq!(Count::get(&capture.settle_duration), 4, "settle samples");
        }
    }
}
