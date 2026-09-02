//! Facilitator trait, cache, pending settlement, and remote HTTP client for
//! the x402 payment protocol.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    test,
    allow(
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "unit tests panic on assertion failure"
    )
)]

mod cache;
mod facilitator;
mod hooks;
mod pending;

#[cfg(test)]
use wiremock as _;

#[cfg(feature = "http-client")]
pub mod http;

pub use cache::{
    DEFAULT_SETTLEMENT_CAPACITY, DEFAULT_SETTLEMENT_TTL, Duplicate, SettlementCache, TtlSet,
};
pub use facilitator::{BoxFuture, DynFacilitator, Facilitator};
pub use hooks::{
    DynFacilitatorHooks, FacilitatorHooks, FailureRecovery, HookDecision, HookedFacilitator,
    SettleContext, VerifyContext,
};
#[cfg(feature = "http-client")]
pub use http::{
    FacilitatorAuthHeaders, FacilitatorClient, FacilitatorClientError, SupportedCache,
    compute_retry_delay,
};
pub use pending::{InMemoryPendingSettlementStore, PENDING_SETTLEMENT_TTL, PendingSettlementStore};
