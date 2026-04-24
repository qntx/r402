#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

// These crates are re-exported behind feature flags and consumed indirectly
// by tracing macros / telemetry instrumentation; the linter cannot see those
// references, so we suppress the unused-crate warning.
#[cfg(feature = "telemetry")]
use tracing as _;
#[cfg(feature = "telemetry")]
use tracing_core as _;

pub mod amount;
pub mod chain;
pub mod error;
pub mod error_reason;
pub mod extensions;
pub mod facilitator;
pub mod hooks;
pub mod payment;
pub mod scheme;
pub mod wire;

#[cfg(feature = "cache")]
#[cfg_attr(docsrs, doc(cfg(feature = "cache")))]
pub mod cache;

pub use error::{ClientError, FacilitatorError, SettlementError, VerificationError};
pub use error_reason::{AsPaymentProblem, ErrorReason, PaymentProblem};
pub use facilitator::{BoxFuture, DynFacilitator, Facilitator};
pub use hooks::{
    DynFacilitatorHooks, FacilitatorHooks, FailureRecovery, HookDecision, HookedFacilitator,
    SettleContext as HookSettleContext, VerifyContext as HookVerifyContext,
};
