#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    test,
    allow(
        unknown_lints,
        clippy::unused_async_trait_impl,
        reason = "in-crate mock impls of AFIT traits have no .await"
    )
)]

//! Algorand chain support for the x402 payment protocol.
//!
//! This crate implements the x402 `"exact"` scheme for Algorand: the buyer
//! builds an atomic transaction group containing an ASA transfer and an
//! optional facilitator-sponsored 0-amount payment that covers the group
//! fee via fee pooling.
//!
//! # Features
//!
//! - **CAIP-2 Addressing**: `algorand:wGHE2Pwdvd7S12BL5FaOP20EGYesN73k`
//!   and `algorand:SGO1GKSzyE7IEPItTxCByw9x8FmnrCDe`
//! - **ASA Payments**: exact transfer of a token (USDC by default)
//! - **Fee pooling**: optional 0-amount fee-payer transaction
//! - **In-process facilitator**: algod REST verify and settle
//!
//! # Feature Flags
//!
//! - `server` — server-side price tag generation
//! - `client` — client-side group construction and signing
//! - `facilitator` — facilitator-side payment verification and settlement
//! - `telemetry` — `tracing` instrumentation

#[cfg(not(any(feature = "client", feature = "facilitator")))]
use base64 as _;
#[cfg(feature = "telemetry")]
use tracing_core as _;

/// Maximum number of top-level transactions in an Algorand atomic group.
pub const MAX_TRANSACTION_GROUP_SIZE: usize = 16;

/// Per-transaction fee cap used to bound facilitator fee-payer spend (µAlgo).
pub const MAX_REASONABLE_FEE_PER_TXN: u64 = 5_000;

/// Default first/last-valid window in rounds (`last_round + this`).
pub const DEFAULT_VALIDITY_ROUNDS: u64 = 1_000;

/// Default number of rounds to wait for confirmation after broadcast.
pub const DEFAULT_WAIT_ROUNDS: u32 = 10;

/// Default USDC decimal precision.
pub const DEFAULT_TOKEN_DECIMALS: u8 = 6;

/// Maximum acceptable fee-payer fee for a group of `group_size` transactions.
#[must_use]
pub fn max_reasonable_group_fee(group_size: usize) -> u64 {
    let size = u64::try_from(group_size).unwrap_or(u64::MAX);
    MAX_REASONABLE_FEE_PER_TXN.saturating_mul(size)
}

pub mod chain;
pub mod exact;

mod networks;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::AlgodClient;
#[cfg(feature = "facilitator")]
pub use chain::AlgorandChainProvider;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::AlgorandSigner;
pub use exact::AlgorandExact;
#[cfg(feature = "client")]
pub use exact::client::AlgorandExactClient;
pub use networks::*;
