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

pub mod chain;
pub mod exact;

mod networks;
#[cfg(feature = "facilitator")]
pub use chain::SOLANA_MAX_COMPUTE_UNIT_PRICE_MICROLAMPORTS;
pub use exact::SolanaExact;
#[cfg(feature = "client")]
pub use exact::client::SolanaExactClient;
pub use networks::*;
