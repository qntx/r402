//! Lifecycle hooks for the x402 payment gate.
//!
//! Hooks allow resource servers to intercept and customize the payment
//! verification and settlement lifecycle. This mirrors the Go SDK's
//! `server_hooks.go` design.
//!
//! ## Hook Types
//!
//! - **Before hooks** — Run before an operation; can abort it.
//! - **After hooks** — Run after a successful operation; errors are logged but don't affect the result.
//! - **Failure hooks** — Run when an operation fails; can recover with a substitute result.

use r402::proto;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Context passed to verify hooks.
#[derive(Debug, Clone)]
pub struct VerifyContext {
    /// The verify request about to be (or already) sent to the facilitator.
    pub request: proto::VerifyRequest,
}

/// Context passed to verify result (after) hooks.
#[derive(Debug, Clone)]
pub struct VerifyResultContext {
    /// Original verify context.
    pub ctx: VerifyContext,
    /// The successful verify response.
    pub result: proto::VerifyResponse,
}

/// Context passed to verify failure hooks.
#[derive(Debug, Clone)]
pub struct VerifyFailureContext {
    /// Original verify context.
    pub ctx: VerifyContext,
    /// The error message.
    pub error: String,
}

/// Context passed to settle hooks.
#[derive(Debug, Clone)]
pub struct SettleContext {
    /// The settle request about to be (or already) sent to the facilitator.
    pub request: proto::SettleRequest,
}

/// Context passed to settle result (after) hooks.
#[derive(Debug, Clone)]
pub struct SettleResultContext {
    /// Original settle context.
    pub ctx: SettleContext,
    /// The successful settle response.
    pub result: proto::SettleResponse,
}

/// Context passed to settle failure hooks.
#[derive(Debug, Clone)]
pub struct SettleFailureContext {
    /// Original settle context.
    pub ctx: SettleContext,
    /// The error message.
    pub error: String,
}

/// Result returned by a "before" hook.
///
/// If `abort` is `true`, the operation is skipped and an error is returned
/// with `reason` as the message.
#[derive(Debug, Clone, Default)]
pub struct BeforeHookResult {
    /// Whether to abort the operation.
    pub abort: bool,
    /// Human-readable reason for aborting.
    pub reason: String,
}

/// Result returned by a verify failure hook.
///
/// If `recovered` is `true`, the `result` is used instead of propagating the error.
#[derive(Debug, Clone)]
pub struct VerifyFailureHookResult {
    /// Whether this hook recovered from the failure.
    pub recovered: bool,
    /// The replacement verify response (only used if `recovered` is `true`).
    pub result: proto::VerifyResponse,
}

/// Result returned by a settle failure hook.
///
/// If `recovered` is `true`, the `result` is used instead of propagating the error.
#[derive(Debug, Clone)]
pub struct SettleFailureHookResult {
    /// Whether this hook recovered from the failure.
    pub recovered: bool,
    /// The replacement settle response (only used if `recovered` is `true`).
    pub result: proto::SettleResponse,
}

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Type alias for a before-verify hook callback.
pub type BeforeVerifyHookFn = dyn Fn(VerifyContext) -> BoxFut<'static, Result<Option<BeforeHookResult>, String>>
    + Send
    + Sync;

/// Type alias for an after-verify hook callback.
pub type AfterVerifyHookFn =
    dyn Fn(VerifyResultContext) -> BoxFut<'static, Result<(), String>> + Send + Sync;

/// Type alias for an on-verify-failure hook callback.
pub type OnVerifyFailureHookFn = dyn Fn(VerifyFailureContext) -> BoxFut<'static, Result<Option<VerifyFailureHookResult>, String>>
    + Send
    + Sync;

/// Type alias for a before-settle hook callback.
pub type BeforeSettleHookFn = dyn Fn(SettleContext) -> BoxFut<'static, Result<Option<BeforeHookResult>, String>>
    + Send
    + Sync;

/// Type alias for an after-settle hook callback.
pub type AfterSettleHookFn =
    dyn Fn(SettleResultContext) -> BoxFut<'static, Result<(), String>> + Send + Sync;

/// Type alias for an on-settle-failure hook callback.
pub type OnSettleFailureHookFn = dyn Fn(SettleFailureContext) -> BoxFut<'static, Result<Option<SettleFailureHookResult>, String>>
    + Send
    + Sync;

/// Collection of lifecycle hooks for the payment gate.
///
/// All hooks are optional and stored as `Arc`-wrapped boxed closures.
/// Multiple hooks of the same type are executed in registration order.
#[derive(Clone, Default)]
pub struct PaygateHooks {
    pub(crate) before_verify: Vec<Arc<BeforeVerifyHookFn>>,
    pub(crate) after_verify: Vec<Arc<AfterVerifyHookFn>>,
    pub(crate) on_verify_failure: Vec<Arc<OnVerifyFailureHookFn>>,
    pub(crate) before_settle: Vec<Arc<BeforeSettleHookFn>>,
    pub(crate) after_settle: Vec<Arc<AfterSettleHookFn>>,
    pub(crate) on_settle_failure: Vec<Arc<OnSettleFailureHookFn>>,
}

impl std::fmt::Debug for PaygateHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaygateHooks")
            .field("before_verify", &self.before_verify.len())
            .field("after_verify", &self.after_verify.len())
            .field("on_verify_failure", &self.on_verify_failure.len())
            .field("before_settle", &self.before_settle.len())
            .field("after_settle", &self.after_settle.len())
            .field("on_settle_failure", &self.on_settle_failure.len())
            .finish()
    }
}

impl PaygateHooks {
    /// Returns `true` if no hooks are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.before_verify.is_empty()
            && self.after_verify.is_empty()
            && self.on_verify_failure.is_empty()
            && self.before_settle.is_empty()
            && self.after_settle.is_empty()
            && self.on_settle_failure.is_empty()
    }

    /// Registers a hook to execute before payment verification.
    ///
    /// If the hook returns `Some(BeforeHookResult { abort: true, .. })`,
    /// verification is skipped and the reason is returned as an error.
    #[must_use]
    pub fn on_before_verify<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(VerifyContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<BeforeHookResult>, String>> + Send + 'static,
    {
        self.before_verify
            .push(Arc::new(move |ctx| Box::pin(hook(ctx))));
        self
    }

    /// Registers a hook to execute after successful payment verification.
    ///
    /// Errors from this hook are logged but do not affect the result.
    #[must_use]
    pub fn on_after_verify<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(VerifyResultContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        self.after_verify
            .push(Arc::new(move |ctx| Box::pin(hook(ctx))));
        self
    }

    /// Registers a hook to execute when payment verification fails.
    ///
    /// If the hook returns `Some(VerifyFailureHookResult { recovered: true, .. })`,
    /// the provided result replaces the error.
    #[must_use]
    pub fn on_verify_failure<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(VerifyFailureContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<VerifyFailureHookResult>, String>> + Send + 'static,
    {
        self.on_verify_failure
            .push(Arc::new(move |ctx| Box::pin(hook(ctx))));
        self
    }

    /// Registers a hook to execute before payment settlement.
    ///
    /// If the hook returns `Some(BeforeHookResult { abort: true, .. })`,
    /// settlement is skipped and the reason is returned as an error.
    #[must_use]
    pub fn on_before_settle<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(SettleContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<BeforeHookResult>, String>> + Send + 'static,
    {
        self.before_settle
            .push(Arc::new(move |ctx| Box::pin(hook(ctx))));
        self
    }

    /// Registers a hook to execute after successful payment settlement.
    ///
    /// Errors from this hook are logged but do not affect the result.
    #[must_use]
    pub fn on_after_settle<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(SettleResultContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        self.after_settle
            .push(Arc::new(move |ctx| Box::pin(hook(ctx))));
        self
    }

    /// Registers a hook to execute when payment settlement fails.
    ///
    /// If the hook returns `Some(SettleFailureHookResult { recovered: true, .. })`,
    /// the provided result replaces the error.
    #[must_use]
    pub fn on_settle_failure<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(SettleFailureContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<SettleFailureHookResult>, String>> + Send + 'static,
    {
        self.on_settle_failure
            .push(Arc::new(move |ctx| Box::pin(hook(ctx))));
        self
    }
}
