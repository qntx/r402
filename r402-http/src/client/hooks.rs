//! Lifecycle hooks for the x402 client payment creation pipeline.
//!
//! Hooks allow applications to intercept and customize the payment
//! creation lifecycle. This mirrors the Go SDK's `client_hooks.go` design.
//!
//! ## Hook Types
//!
//! - **Before hooks** — Run before payment creation; can abort it.
//! - **After hooks** — Run after successful payment creation; errors are logged but don't affect the result.
//! - **Failure hooks** — Run when payment creation fails; can recover with a substitute payload.

use http::HeaderMap;
use r402::proto;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Context passed to before-payment-creation hooks.
#[derive(Debug, Clone)]
pub struct PaymentCreationContext {
    /// The parsed payment requirements from the 402 response.
    pub payment_required: proto::PaymentRequired,
}

/// Context passed to after-payment-creation hooks.
#[derive(Debug, Clone)]
pub struct PaymentCreatedContext {
    /// Original creation context.
    pub ctx: PaymentCreationContext,
    /// The payment headers that will be sent with the retry request.
    pub headers: HeaderMap,
}

/// Context passed to payment creation failure hooks.
#[derive(Debug, Clone)]
pub struct PaymentCreationFailureContext {
    /// Original creation context.
    pub ctx: PaymentCreationContext,
    /// The error message.
    pub error: String,
}

/// Result returned by a before-payment-creation hook.
///
/// If `abort` is `true`, payment creation is skipped and the original
/// 402 response is returned to the caller.
#[derive(Debug, Clone, Default)]
pub struct BeforePaymentCreationHookResult {
    /// Whether to abort payment creation.
    pub abort: bool,
    /// Human-readable reason for aborting.
    pub reason: String,
}

/// Result returned by a payment creation failure hook.
///
/// If `recovered` is `true`, the `headers` are used in place of the failed result.
#[derive(Debug, Clone)]
pub struct PaymentCreationFailureHookResult {
    /// Whether this hook recovered from the failure.
    pub recovered: bool,
    /// Replacement payment headers (only used if `recovered` is `true`).
    pub headers: HeaderMap,
}

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Type alias for a before-payment-creation hook callback.
pub type BeforePaymentCreationHookFn = dyn Fn(
        PaymentCreationContext,
    ) -> BoxFut<'static, Result<Option<BeforePaymentCreationHookResult>, String>>
    + Send
    + Sync;

/// Type alias for an after-payment-creation hook callback.
pub type AfterPaymentCreationHookFn =
    dyn Fn(PaymentCreatedContext) -> BoxFut<'static, Result<(), String>> + Send + Sync;

/// Type alias for a payment creation failure hook callback.
pub type OnPaymentCreationFailureHookFn = dyn Fn(
        PaymentCreationFailureContext,
    ) -> BoxFut<'static, Result<Option<PaymentCreationFailureHookResult>, String>>
    + Send
    + Sync;

/// Collection of lifecycle hooks for the client payment creation pipeline.
///
/// All hooks are optional and stored as `Arc`-wrapped boxed closures.
/// Multiple hooks of the same type are executed in registration order.
#[derive(Clone, Default)]
pub struct ClientHooks {
    pub(crate) before_payment_creation: Vec<Arc<BeforePaymentCreationHookFn>>,
    pub(crate) after_payment_creation: Vec<Arc<AfterPaymentCreationHookFn>>,
    pub(crate) on_payment_creation_failure: Vec<Arc<OnPaymentCreationFailureHookFn>>,
}

impl std::fmt::Debug for ClientHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientHooks")
            .field(
                "before_payment_creation",
                &self.before_payment_creation.len(),
            )
            .field("after_payment_creation", &self.after_payment_creation.len())
            .field(
                "on_payment_creation_failure",
                &self.on_payment_creation_failure.len(),
            )
            .finish()
    }
}

impl ClientHooks {
    /// Returns `true` if no hooks are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.before_payment_creation.is_empty()
            && self.after_payment_creation.is_empty()
            && self.on_payment_creation_failure.is_empty()
    }

    /// Registers a hook to execute before payment creation.
    ///
    /// If the hook returns `Some(BeforePaymentCreationHookResult { abort: true, .. })`,
    /// payment creation is skipped.
    #[must_use]
    pub fn on_before_payment_creation<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(PaymentCreationContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<BeforePaymentCreationHookResult>, String>>
            + Send
            + 'static,
    {
        self.before_payment_creation
            .push(Arc::new(move |ctx| Box::pin(hook(ctx))));
        self
    }

    /// Registers a hook to execute after successful payment creation.
    ///
    /// Errors from this hook are logged but do not affect the result.
    #[must_use]
    pub fn on_after_payment_creation<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(PaymentCreatedContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        self.after_payment_creation
            .push(Arc::new(move |ctx| Box::pin(hook(ctx))));
        self
    }

    /// Registers a hook to execute when payment creation fails.
    ///
    /// If the hook returns `Some(PaymentCreationFailureHookResult { recovered: true, .. })`,
    /// the provided headers replace the error.
    #[must_use]
    pub fn on_payment_creation_failure<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(PaymentCreationFailureContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<PaymentCreationFailureHookResult>, String>>
            + Send
            + 'static,
    {
        self.on_payment_creation_failure
            .push(Arc::new(move |ctx| Box::pin(hook(ctx))));
        self
    }
}
