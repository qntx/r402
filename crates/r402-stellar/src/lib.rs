#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    test,
    allow(
        unknown_lints,
        clippy::unused_async_trait_impl,
        reason = "in-crate mock impls of AFIT traits have no .await"
    )
)]

//! Stellar chain support for the x402 payment protocol.
//!
//! This crate implements the x402 `"exact"` scheme for Stellar: the buyer
//! signs Soroban authorization entries for one SEP-41 `transfer`, and the
//! facilitator is the transaction source / fee sponsor.
//!
//! # Features
//!
//! - **CAIP-2 Addressing**: `stellar:pubnet` and `stellar:testnet`
//! - **SEP-41 Payments**: exact `transfer` of a Soroban token (USDC by default)
//! - **Auth-entry signing**: the client never spends a sequence number
//! - **In-process facilitator**: RPC verify, rebuild, submit, and confirm
//!
//! # Feature Flags
//!
//! - `server` — server-side price tag generation
//! - `client` — client-side authorization-entry signing
//! - `facilitator` — facilitator-side payment verification and settlement
//! - `telemetry` — `tracing` instrumentation

#[cfg(feature = "telemetry")]
use tracing_core as _;

/// Default USDC decimal precision on Stellar (SEP-41).
pub const DEFAULT_TOKEN_DECIMALS: u8 = 7;

/// Inclusion buffer in stroops (`BASE_FEE` in the official Stellar SDK).
pub const BASE_FEE_STROOPS: u32 = 100;

/// Safety ceiling for simulation-derived settlement fees (stroops).
pub const DEFAULT_MAX_TRANSACTION_FEE_STROOPS: u32 = 50_000;

/// Fallback ledger close time when Horizon is unavailable.
pub const DEFAULT_ESTIMATED_LEDGER_SECONDS: u64 = 5;

/// Ledger-skew tolerance applied to auth-entry expiration.
pub const SIGNATURE_EXPIRATION_LEDGER_TOLERANCE: u32 = 2;

/// Default `maxTimeoutSeconds` when the requirement omits it.
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 60;

/// Number of recent Horizon ledgers sampled for close-time estimation.
pub const HORIZON_LEDGERS_SAMPLE_SIZE: u32 = 20;

/// Dummy source used by the client so only auth entries are signed.
///
/// Matches `@stellar/stellar-sdk` `NULL_ACCOUNT`.
pub const NULL_ACCOUNT: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";

/// SEP-41 transfer method name.
pub const TRANSFER_FUNCTION: &str = "transfer";

pub mod chain;
pub mod exact;

mod networks;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::StellarJsonRpc;
#[cfg(feature = "facilitator")]
pub use chain::{StellarChainProvider, StellarFacilitatorError};
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::{StellarRpc, StellarRpcError, StellarSigner};
pub use exact::StellarExact;
#[cfg(feature = "client")]
pub use exact::client::StellarExactClient;
pub use networks::*;

/// `ledgerTimeout = ceil(maxTimeoutSeconds / estimatedLedgerSeconds)`.
#[must_use]
pub fn timeout_ledgers(max_timeout_seconds: u64, estimated_ledger_seconds: u64) -> u32 {
    let estimated = estimated_ledger_seconds.max(1);
    let ledgers = max_timeout_seconds.div_ceil(estimated);
    u32::try_from(ledgers).unwrap_or(u32::MAX)
}

/// SHA-256 of the network passphrase (Stellar network id).
#[cfg(any(feature = "client", feature = "facilitator"))]
#[must_use]
pub fn network_id(passphrase: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(passphrase.as_bytes()).into()
}
