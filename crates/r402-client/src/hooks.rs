//! Buyer lifecycle hooks around payment creation and paid responses.

use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::pin::Pin;

use r402_protocol::{PaymentRequired, SettleResponse};

use crate::register::PaymentClient;
use crate::select::PaymentSelector;

/// Boxed future used by [`DynClientHooks`].
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Decision returned by "before" hooks to control whether the operation proceeds.
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

/// Outcome returned by "on failure" hooks.
#[derive(Debug)]
#[non_exhaustive]
pub enum FailureRecovery<T> {
    /// No recovery — propagate the original error.
    Propagate,
    /// The hook produced a substitute success result.
    Recovered(T),
}

/// Context for payment-creation hooks.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PaymentCreationContext {
    /// Parsed payment requirements from the 402 challenge.
    pub payment_required: PaymentRequired,
}

impl PaymentCreationContext {
    /// Constructs a creation context from a 402 challenge.
    #[must_use]
    pub const fn new(payment_required: PaymentRequired) -> Self {
        Self { payment_required }
    }
}

/// Result of successful payment creation (transport-agnostic).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CreatedPayment {
    /// Base64-encoded payment payload for `Payment-Signature` / MCP meta.
    pub signed_payload: String,
    /// Challenge that was paid.
    pub payment_required: PaymentRequired,
}

impl CreatedPayment {
    /// Constructs a created-payment record.
    #[must_use]
    pub fn new(signed_payload: impl Into<String>, payment_required: PaymentRequired) -> Self {
        Self {
            signed_payload: signed_payload.into(),
            payment_required,
        }
    }
}

/// Context delivered after a paid request completes.
///
/// Typically one of `settle_response` or `corrective_payment_required` is set:
/// settle for a paid success with `Payment-Response`; corrective 402 when the
/// server rejected with a new `Payment-Required`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PaymentResponseContext {
    /// Original 402 challenge used to build the payment.
    pub payment_required: PaymentRequired,
    /// Signed payload that was submitted.
    pub signed_payload: String,
    /// Parsed settle outcome when present.
    pub settle_response: Option<SettleResponse>,
    /// Corrective `Payment-Required` when the paid retry returned 402.
    pub corrective_payment_required: Option<PaymentRequired>,
}

impl PaymentResponseContext {
    /// Constructs a payment-response context.
    #[must_use]
    pub fn new(payment_required: PaymentRequired, signed_payload: impl Into<String>) -> Self {
        Self {
            payment_required,
            signed_payload: signed_payload.into(),
            settle_response: None,
            corrective_payment_required: None,
        }
    }

    /// Attaches a settle response.
    #[must_use]
    pub fn with_settle_response(mut self, settle: SettleResponse) -> Self {
        self.settle_response = Some(settle);
        self
    }

    /// Attaches a corrective payment-required challenge.
    #[must_use]
    pub fn with_corrective_payment_required(mut self, required: PaymentRequired) -> Self {
        self.corrective_payment_required = Some(required);
        self
    }
}

/// Result of [`ClientHooks::on_payment_response`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PaymentResponseResult {
    /// When `true`, the transport should retry once with a freshly built payload.
    pub recovered: bool,
}

impl PaymentResponseResult {
    /// No recovery — continue with the response as-is.
    #[must_use]
    pub const fn continue_() -> Self {
        Self { recovered: false }
    }

    /// Signal one corrective retry.
    #[must_use]
    pub const fn recovered() -> Self {
        Self { recovered: true }
    }
}

/// Lifecycle hooks for the payment client.
///
/// All methods default to no-ops. Override only what you need.
pub trait ClientHooks: Send + Sync {
    /// Runs before payment payload creation. Abort skips signing.
    fn before_payment_creation<'a>(
        &'a self,
        _ctx: &'a PaymentCreationContext,
    ) -> impl Future<Output = HookDecision> + Send + 'a {
        async { HookDecision::Continue }
    }

    /// Runs after a payment payload is successfully created.
    fn after_payment_creation<'a>(
        &'a self,
        _ctx: &'a PaymentCreationContext,
        _created: &'a CreatedPayment,
    ) -> impl Future<Output = ()> + Send + 'a {
        async {}
    }

    /// Runs when payment creation fails. May recover with a substitute payload.
    fn on_payment_creation_failure<'a>(
        &'a self,
        _ctx: &'a PaymentCreationContext,
        _error: &'a str,
    ) -> impl Future<Output = FailureRecovery<CreatedPayment>> + Send + 'a {
        async { FailureRecovery::Propagate }
    }

    /// Runs after each paid response (settle success or corrective 402).
    ///
    /// Returning [`PaymentResponseResult::recovered`] asks the transport to
    /// retry once with a freshly built payment payload.
    fn on_payment_response<'a>(
        &'a self,
        _ctx: &'a PaymentResponseContext,
    ) -> impl Future<Output = PaymentResponseResult> + Send + 'a {
        async { PaymentResponseResult::continue_() }
    }
}

/// Object-safe erasure of [`ClientHooks`].
pub trait DynClientHooks: Send + Sync {
    /// See [`ClientHooks::before_payment_creation`].
    fn before_payment_creation<'a>(
        &'a self,
        ctx: &'a PaymentCreationContext,
    ) -> Pin<Box<dyn Future<Output = HookDecision> + Send + 'a>>;

    /// See [`ClientHooks::after_payment_creation`].
    fn after_payment_creation<'a>(
        &'a self,
        ctx: &'a PaymentCreationContext,
        created: &'a CreatedPayment,
    ) -> BoxFuture<'a, ()>;

    /// See [`ClientHooks::on_payment_creation_failure`].
    fn on_payment_creation_failure<'a>(
        &'a self,
        ctx: &'a PaymentCreationContext,
        error: &'a str,
    ) -> BoxFuture<'a, FailureRecovery<CreatedPayment>>;

    /// See [`ClientHooks::on_payment_response`].
    fn on_payment_response<'a>(
        &'a self,
        ctx: &'a PaymentResponseContext,
    ) -> Pin<Box<dyn Future<Output = PaymentResponseResult> + Send + 'a>>;
}

impl<T: ClientHooks + ?Sized> DynClientHooks for T {
    fn before_payment_creation<'a>(
        &'a self,
        ctx: &'a PaymentCreationContext,
    ) -> Pin<Box<dyn Future<Output = HookDecision> + Send + 'a>> {
        Box::pin(<Self as ClientHooks>::before_payment_creation(self, ctx))
    }

    fn after_payment_creation<'a>(
        &'a self,
        ctx: &'a PaymentCreationContext,
        created: &'a CreatedPayment,
    ) -> BoxFuture<'a, ()> {
        Box::pin(<Self as ClientHooks>::after_payment_creation(
            self, ctx, created,
        ))
    }

    fn on_payment_creation_failure<'a>(
        &'a self,
        ctx: &'a PaymentCreationContext,
        error: &'a str,
    ) -> BoxFuture<'a, FailureRecovery<CreatedPayment>> {
        Box::pin(<Self as ClientHooks>::on_payment_creation_failure(
            self, ctx, error,
        ))
    }

    fn on_payment_response<'a>(
        &'a self,
        ctx: &'a PaymentResponseContext,
    ) -> Pin<Box<dyn Future<Output = PaymentResponseResult> + Send + 'a>> {
        Box::pin(<Self as ClientHooks>::on_payment_response(self, ctx))
    }
}

impl Debug for dyn DynClientHooks {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("DynClientHooks")
    }
}

impl<S: PaymentSelector> PaymentClient<S> {
    /// Dispatches [`ClientHooks::on_payment_response`] for every hook.
    ///
    /// First `recovered: true` wins; remaining hooks still run.
    pub async fn handle_payment_response(
        &self,
        ctx: &PaymentResponseContext,
    ) -> PaymentResponseResult {
        let mut recovered = false;
        for hook in &self.hooks {
            let result = hook.on_payment_response(ctx).await;
            if result.recovered {
                recovered = true;
            }
        }
        if recovered {
            PaymentResponseResult::recovered()
        } else {
            PaymentResponseResult::continue_()
        }
    }
}
