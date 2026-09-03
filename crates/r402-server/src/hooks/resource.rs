//! Resource-server lifecycle hooks.

use std::fmt::{self, Debug, Formatter};
use std::future::Future;

use compact_str::CompactString;
use r402_facilitator::{BoxFuture, FailureRecovery};
use r402_protocol::error::FacilitatorError;
use r402_protocol::payment::{
    Extensions, PaymentPayload, PaymentRequirements, SettleResponse, VerifyResponse,
};

use crate::payment_flow::SettlePhase;

/// Wire payment payload with typed requirements and opaque scheme body.
pub type WirePaymentPayload = PaymentPayload<PaymentRequirements, serde_json::Value>;

/// Why a verified payment was canceled before settlement.
///
/// Wire-stable `snake_case` labels: `handler_threw` / `handler_failed` /
/// `after_verify_aborted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CancelReason {
    /// Protected handler panicked or returned a transport error.
    HandlerThrew,
    /// Protected handler completed with a failing status (≥ 400).
    HandlerFailed,
    /// An `after_verify` hook aborted after a successful verify.
    AfterVerifyAborted,
}

impl CancelReason {
    /// Stable machine-readable label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HandlerThrew => "handler_threw",
            Self::HandlerFailed => "handler_failed",
            Self::AfterVerifyAborted => "after_verify_aborted",
        }
    }
}

impl fmt::Display for CancelReason {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Decision from a "before verify/settle" hook.
#[derive(Debug)]
#[non_exhaustive]
pub enum BeforeOpDecision<T> {
    /// Proceed with the facilitator call.
    Continue,
    /// Abort the operation with a structured reason.
    Abort {
        /// Machine-readable reason.
        reason: String,
        /// Human-readable description.
        message: String,
    },
    /// Short-circuit: use this local result instead of calling the facilitator.
    Skip {
        /// Locally produced response.
        result: T,
    },
}

/// In-process directive when an after-verify hook skips the resource handler.
///
/// Never appears on the facilitator wire; transports may use `body` as the
/// success response when settling inline.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct SkipHandlerDirective {
    /// Optional content type for the transport response body.
    pub content_type: Option<String>,
    /// Optional JSON body for the transport success response.
    pub body: Option<serde_json::Value>,
}

impl SkipHandlerDirective {
    /// Empty directive (settle inline, default success body).
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            content_type: None,
            body: None,
        }
    }

    /// Builder: attach a content type.
    #[must_use]
    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    /// Builder: attach a JSON body.
    #[must_use]
    pub fn with_body(mut self, body: serde_json::Value) -> Self {
        self.body = Some(body);
        self
    }
}

/// Decision from an after-verify hook.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum AfterVerifyDecision {
    /// Continue to the resource handler (default).
    #[default]
    Continue,
    /// Fail closed: fire cancel (`after_verify_aborted`) and reject payment.
    Abort {
        /// Machine-readable reason.
        reason: String,
        /// Human-readable description.
        message: String,
    },
    /// Bypass the resource handler; transport should settle inline.
    SkipHandler {
        /// Optional success body for the transport.
        response: SkipHandlerDirective,
    },
}

/// Shared context for resource-server payment hooks.
#[derive(Clone)]
#[non_exhaustive]
pub struct PaymentHookContext {
    /// Client payment payload.
    pub payload: WirePaymentPayload,
    /// Matched payment requirements.
    pub requirements: PaymentRequirements,
}

impl Debug for PaymentHookContext {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaymentHookContext")
            .field("requirements", &self.requirements)
            .finish_non_exhaustive()
    }
}

impl PaymentHookContext {
    /// Constructs a payment hook context.
    #[must_use]
    pub const fn new(payload: WirePaymentPayload, requirements: PaymentRequirements) -> Self {
        Self {
            payload,
            requirements,
        }
    }
}

/// Context for after-verify hooks (includes facilitator result).
#[derive(Clone)]
#[non_exhaustive]
pub struct VerifyResultContext {
    /// Base payment context.
    pub payment: PaymentHookContext,
    /// Facilitator (or skip/recover) verify response — always a success path
    /// entry (`Valid` or recovered result passed to after hooks).
    pub result: VerifyResponse,
}

impl Debug for VerifyResultContext {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerifyResultContext")
            .field("payment", &self.payment)
            .field("result_valid", &self.result.is_valid())
            .finish_non_exhaustive()
    }
}

/// Context for before-settle / settle-failure hooks.
#[derive(Clone)]
#[non_exhaustive]
pub struct SettleContext {
    /// Base payment context.
    pub payment: PaymentHookContext,
    /// Extension IDs declared on the 402 for this settle.
    pub declared_extensions: Extensions,
    /// Which settle invocation is running.
    pub phase: SettlePhase,
    /// Resource URL from the 402 / request, when known.
    pub resource_url: Option<CompactString>,
}

impl Debug for SettleContext {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SettleContext")
            .field("phase", &self.phase)
            .field("payment", &self.payment)
            .finish_non_exhaustive()
    }
}

impl SettleContext {
    /// Constructs a settle context with empty declared extensions.
    #[must_use]
    pub fn new(payment: PaymentHookContext, phase: SettlePhase) -> Self {
        Self {
            payment,
            declared_extensions: Extensions::new(),
            phase,
            resource_url: None,
        }
    }
}

/// Context for after-settle hooks.
#[derive(Clone)]
#[non_exhaustive]
pub struct SettleResultContext {
    /// Settle invocation that produced `result`.
    pub settle: SettleContext,
    /// Facilitator settle response.
    pub result: SettleResponse,
}

impl Debug for SettleResultContext {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SettleResultContext")
            .field("settle", &self.settle)
            .field("result_success", &self.result.is_success())
            .finish_non_exhaustive()
    }
}

/// Context for verified-payment cancellation.
#[derive(Clone)]
#[non_exhaustive]
pub struct VerifiedPaymentCanceledContext {
    /// Base payment context.
    pub payment: PaymentHookContext,
    /// Cancellation reason.
    pub reason: CancelReason,
    /// Optional error message from the handler / hook.
    pub error: Option<String>,
    /// Optional transport status from a failed handler response.
    pub response_status: Option<u16>,
    /// Settle phases already completed for this payment.
    pub settled_phases: Vec<SettlePhase>,
}

impl Debug for VerifiedPaymentCanceledContext {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerifiedPaymentCanceledContext")
            .field("reason", &self.reason)
            .field("response_status", &self.response_status)
            .field("settled_phases", &self.settled_phases)
            .finish_non_exhaustive()
    }
}

impl VerifiedPaymentCanceledContext {
    /// Constructs a cancel context.
    #[must_use]
    pub const fn new(
        payment: PaymentHookContext,
        reason: CancelReason,
        settled_phases: Vec<SettlePhase>,
    ) -> Self {
        Self {
            payment,
            reason,
            error: None,
            response_status: None,
            settled_phases,
        }
    }
}

/// Lifecycle hooks for the resource server (transport-agnostic).
///
/// All methods default to no-ops. Override only what you need.
pub trait ResourceServerHooks: Send + Sync {
    /// Runs before facilitator verify.
    fn before_verify<'a>(
        &'a self,
        _ctx: &'a PaymentHookContext,
    ) -> impl Future<Output = BeforeOpDecision<VerifyResponse>> + Send + 'a {
        async { BeforeOpDecision::Continue }
    }

    /// Runs after a successful verify (including skip / failure recovery).
    fn after_verify<'a>(
        &'a self,
        _ctx: &'a VerifyResultContext,
    ) -> impl Future<Output = AfterVerifyDecision> + Send + 'a {
        async { AfterVerifyDecision::Continue }
    }

    /// Runs when facilitator verify returns an error.
    fn on_verify_failure<'a>(
        &'a self,
        _ctx: &'a PaymentHookContext,
        _error: &'a FacilitatorError,
    ) -> impl Future<Output = FailureRecovery<VerifyResponse>> + Send + 'a {
        async { FailureRecovery::Propagate }
    }

    /// Runs before facilitator settle.
    fn before_settle<'a>(
        &'a self,
        _ctx: &'a SettleContext,
    ) -> impl Future<Output = BeforeOpDecision<SettleResponse>> + Send + 'a {
        async { BeforeOpDecision::Continue }
    }

    /// Runs after a successful settle (including skip / failure recovery).
    fn after_settle<'a>(
        &'a self,
        _ctx: &'a SettleResultContext,
    ) -> impl Future<Output = ()> + Send + 'a {
        async {}
    }

    /// Runs when facilitator settle returns an error.
    fn on_settle_failure<'a>(
        &'a self,
        _ctx: &'a SettleContext,
        _error: &'a FacilitatorError,
    ) -> impl Future<Output = FailureRecovery<SettleResponse>> + Send + 'a {
        async { FailureRecovery::Propagate }
    }

    /// Runs when a verified payment will not be settled.
    fn on_verified_payment_canceled<'a>(
        &'a self,
        _ctx: &'a VerifiedPaymentCanceledContext,
    ) -> impl Future<Output = ()> + Send + 'a {
        async {}
    }
}

/// Object-safe erasure of [`ResourceServerHooks`].
pub trait DynResourceServerHooks: Send + Sync {
    /// See [`ResourceServerHooks::before_verify`].
    fn before_verify<'a>(
        &'a self,
        ctx: &'a PaymentHookContext,
    ) -> BoxFuture<'a, BeforeOpDecision<VerifyResponse>>;

    /// See [`ResourceServerHooks::after_verify`].
    fn after_verify<'a>(
        &'a self,
        ctx: &'a VerifyResultContext,
    ) -> BoxFuture<'a, AfterVerifyDecision>;

    /// See [`ResourceServerHooks::on_verify_failure`].
    fn on_verify_failure<'a>(
        &'a self,
        ctx: &'a PaymentHookContext,
        error: &'a FacilitatorError,
    ) -> BoxFuture<'a, FailureRecovery<VerifyResponse>>;

    /// See [`ResourceServerHooks::before_settle`].
    fn before_settle<'a>(
        &'a self,
        ctx: &'a SettleContext,
    ) -> BoxFuture<'a, BeforeOpDecision<SettleResponse>>;

    /// See [`ResourceServerHooks::after_settle`].
    fn after_settle<'a>(&'a self, ctx: &'a SettleResultContext) -> BoxFuture<'a, ()>;

    /// See [`ResourceServerHooks::on_settle_failure`].
    fn on_settle_failure<'a>(
        &'a self,
        ctx: &'a SettleContext,
        error: &'a FacilitatorError,
    ) -> BoxFuture<'a, FailureRecovery<SettleResponse>>;

    /// See [`ResourceServerHooks::on_verified_payment_canceled`].
    fn on_verified_payment_canceled<'a>(
        &'a self,
        ctx: &'a VerifiedPaymentCanceledContext,
    ) -> BoxFuture<'a, ()>;
}

impl<T: ResourceServerHooks + ?Sized> DynResourceServerHooks for T {
    fn before_verify<'a>(
        &'a self,
        ctx: &'a PaymentHookContext,
    ) -> BoxFuture<'a, BeforeOpDecision<VerifyResponse>> {
        Box::pin(<Self as ResourceServerHooks>::before_verify(self, ctx))
    }

    fn after_verify<'a>(
        &'a self,
        ctx: &'a VerifyResultContext,
    ) -> BoxFuture<'a, AfterVerifyDecision> {
        Box::pin(<Self as ResourceServerHooks>::after_verify(self, ctx))
    }

    fn on_verify_failure<'a>(
        &'a self,
        ctx: &'a PaymentHookContext,
        error: &'a FacilitatorError,
    ) -> BoxFuture<'a, FailureRecovery<VerifyResponse>> {
        Box::pin(<Self as ResourceServerHooks>::on_verify_failure(
            self, ctx, error,
        ))
    }

    fn before_settle<'a>(
        &'a self,
        ctx: &'a SettleContext,
    ) -> BoxFuture<'a, BeforeOpDecision<SettleResponse>> {
        Box::pin(<Self as ResourceServerHooks>::before_settle(self, ctx))
    }

    fn after_settle<'a>(&'a self, ctx: &'a SettleResultContext) -> BoxFuture<'a, ()> {
        Box::pin(<Self as ResourceServerHooks>::after_settle(self, ctx))
    }

    fn on_settle_failure<'a>(
        &'a self,
        ctx: &'a SettleContext,
        error: &'a FacilitatorError,
    ) -> BoxFuture<'a, FailureRecovery<SettleResponse>> {
        Box::pin(<Self as ResourceServerHooks>::on_settle_failure(
            self, ctx, error,
        ))
    }

    fn on_verified_payment_canceled<'a>(
        &'a self,
        ctx: &'a VerifiedPaymentCanceledContext,
    ) -> BoxFuture<'a, ()> {
        Box::pin(<Self as ResourceServerHooks>::on_verified_payment_canceled(
            self, ctx,
        ))
    }
}
