#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    test,
    allow(
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::missing_assert_message,
        reason = "unit tests panic on assertion failure"
    )
)]

//! Solana chain support for the x402 payment protocol.
//!
//! Exact-scheme SPL Token `TransferChecked` payments with pre-signed
//! authorization, fee-payer-bound facilitator submission, and settlement
//! deduplication.

#[cfg(not(any(feature = "client", feature = "server", feature = "facilitator")))]
use serde_json as _;
#[cfg(all(test, not(feature = "facilitator")))]
use tokio as _;

/// Recommended hard cap for `max_compute_unit_price` (micro-lamports).
///
/// Aligned with the Go reference SDK
/// (`mechanisms/svm/exact/facilitator/scheme.go: maxComputeUnitPriceMicroLamports`)
/// so that any client transaction passing Go's verifier passes r402's, and
/// vice versa. A value of 5 000 000 micro-lamports per CU corresponds to
/// roughly 0.005 SOL per million CU, well above realistic priority-fee
/// surge prices on Solana mainnet, while bounding the worst-case fee a
/// fee-paying facilitator can be tricked into paying.
pub const SOLANA_MAX_COMPUTE_UNIT_PRICE_MICROLAMPORTS: u64 = 5_000_000;

pub mod chain;
pub mod exact;

mod networks;
pub use exact::SolanaExact;
#[cfg(feature = "client")]
pub use exact::client::SolanaExactClient;
pub use networks::*;
