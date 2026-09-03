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
//! authorization, and `upto` payment-channel escrow.

#[cfg(all(test, not(feature = "facilitator")))]
use r402_facilitator as _;
#[cfg(all(test, not(feature = "server")))]
use r402_server as _;
#[cfg(not(any(feature = "client", feature = "server", feature = "facilitator")))]
use serde_json as _;
#[cfg(all(test, not(any(feature = "server", feature = "facilitator"))))]
use solana_keypair as _;
#[cfg(all(
    test,
    not(any(feature = "client", feature = "server", feature = "facilitator"))
))]
use solana_signature as _;
#[cfg(all(
    test,
    not(any(feature = "client", feature = "server", feature = "facilitator"))
))]
use solana_signer as _;
#[cfg(all(test, not(any(feature = "facilitator", feature = "server"))))]
use tokio as _;

pub mod chain;
pub mod exact;
pub mod upto;

mod networks;
#[cfg(feature = "facilitator")]
pub use chain::SOLANA_MAX_COMPUTE_UNIT_PRICE_MICROLAMPORTS;
pub use exact::SolanaExact;
#[cfg(feature = "client")]
pub use exact::client::SolanaExactClient;
pub use networks::*;
pub use upto::SolanaUpto;
#[cfg(feature = "client")]
pub use upto::SolanaUptoClient;
#[cfg(feature = "facilitator")]
pub use upto::SolanaUptoFacilitator;
#[cfg(feature = "server")]
pub use upto::UptoSvmScheme;
