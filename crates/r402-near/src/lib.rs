#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    test,
    allow(
        unknown_lints,
        clippy::unused_async_trait_impl,
        reason = "in-crate mock impls of AFIT traits have no .await"
    )
)]

//! NEAR chain support for the x402 payment protocol.
//!
//! This crate implements the x402 `"exact"` scheme for NEAR: the buyer signs a
//! NEP-366 `SignedDelegate` that authorizes one NEP-141 `ft_transfer`, and a
//! facilitator-sponsored relayer submits the outer transaction.
//!
//! # Features
//!
//! - **CAIP-2 Addressing**: `near:mainnet` and `near:testnet`
//! - **NEP-141 Payments**: exact `ft_transfer` of a token (USDC by default)
//! - **NEP-366 Meta-transactions**: client signs a delegate; the relayer pays gas
//! - **In-process facilitator**: JSON-RPC verify and settle
//!
//! # Feature Flags
//!
//! - `server` — server-side price tag generation
//! - `client` — client-side delegate signing
//! - `facilitator` — facilitator-side payment verification and settlement
//! - `telemetry` — `tracing` instrumentation

#[cfg(not(any(feature = "client", feature = "facilitator")))]
use base64 as _;
#[cfg(feature = "telemetry")]
use tracing_core as _;

/// Default gas attached to the delegated `ft_transfer` (30 `TGas`).
pub const DEFAULT_FT_TRANSFER_GAS: u64 = 30_000_000_000_000;

/// Conservative cap on sponsored gas to protect relayers (100 `TGas`).
pub const DEFAULT_MAX_SPONSORED_GAS: u64 = 100_000_000_000_000;

/// NEP-141 requires exactly 1 yoctoNEAR attached to `ft_transfer`.
pub const ONE_YOCTO: u128 = 1;

/// Deterministic timeout mapping (spec §5): `estimatedBlockSeconds = 1`.
pub const ESTIMATED_BLOCK_SECONDS: u64 = 1;

/// NEAR delegate-action nonce upper-bound multiplier
/// (`ACCESS_KEY_NONCE_RANGE_MULTIPLIER`).
pub const NONCE_RANGE_MULTIPLIER: u64 = 1_000_000;

/// Base58 representation of an all-zero 32-byte code hash.
pub const EMPTY_CONTRACT_CODE_HASH: &str = "11111111111111111111111111111111";

/// NEP-141 transfer method.
pub const FT_TRANSFER_METHOD: &str = "ft_transfer";

/// Default USDC decimal precision.
pub const DEFAULT_TOKEN_DECIMALS: u8 = 6;

pub mod chain;
pub mod exact;

mod networks;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::NearJsonRpc;
#[cfg(feature = "facilitator")]
pub use chain::{NearChainProvider, NearRelayer};
pub use exact::NearExact;
#[cfg(feature = "client")]
pub use exact::client::{NearExactClient, NearSigner};
pub use networks::*;

/// `timeoutBlocks = max(1, ceil(maxTimeoutSeconds / estimatedBlockSeconds))`.
#[must_use]
pub const fn timeout_blocks(max_timeout_seconds: u64) -> u64 {
    let blocks = max_timeout_seconds.div_ceil(ESTIMATED_BLOCK_SECONDS);
    if blocks == 0 { 1 } else { blocks }
}
