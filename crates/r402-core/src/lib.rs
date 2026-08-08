#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]
// Tests exercise serde_json values via `value["key"]` indexing which is the
// idiomatic way to write assertions; the panic-on-missing-key behaviour is
// desirable there. Production code never uses `IndexMut` on arbitrary JSON.
#![cfg_attr(
    test,
    allow(
        clippy::indexing_slicing,
        clippy::panic,
        reason = "tests rely on panic-on-failure for simpler assertions"
    )
)]

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
pub mod resource_server;
pub mod scheme;
pub mod wire;

pub use resource_server::{ResourceServer, WirePaymentPayload};

#[cfg(feature = "cache")]
#[cfg_attr(docsrs, doc(cfg(feature = "cache")))]
pub mod cache;

#[cfg(feature = "metrics")]
#[cfg_attr(docsrs, doc(cfg(feature = "metrics")))]
pub mod metrics;

// `proptest` is consumed by integration tests under `tests/`, but the
// lib's own test target also links every dev-dep and would otherwise
// trip `unused_crate_dependencies` because no unit test references it.
// The empty re-import silences that lint without affecting binary size.
pub use error::{ClientError, FacilitatorError, SettlementError, VerificationError};
pub use error_reason::{AsPaymentProblem, ErrorReason, PaymentProblem};
pub use facilitator::{BoxFuture, DynFacilitator, Facilitator};
pub use hooks::{
    DynFacilitatorHooks, FacilitatorHooks, FailureRecovery, HookDecision, HookedFacilitator,
    SettleContext as HookSettleContext, VerifyContext as HookVerifyContext,
};
#[cfg(test)]
use proptest as _;
