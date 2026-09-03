//! Keeta chain support for the x402 payment protocol.
//!
//! Exact scheme: the buyer signs a block containing one `SEND` operation, and
//! a facilitator fee-payer transmits it with `TransmitOptions::with_fee_signer`.
//!
//! # Features
//!
//! - `server` — [`KeetaExact::price_tag`]
//! - `client` — [`KeetaExactClient`] `SEND` block signing
//! - `facilitator` — in-process [`KeetaExactFacilitator::try_new`]
//! - `telemetry` — tracing spans on verify/settle

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    test,
    allow(
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::missing_assert_message,
        clippy::unused_async_trait_impl,
        reason = "unit tests panic on assertion failure; mock AFIT impls have no .await"
    )
)]

#[cfg(not(any(feature = "client", feature = "facilitator")))]
use base64 as _;
#[cfg(all(test, not(any(feature = "client", feature = "facilitator"))))]
use keetanetwork_block as _;
#[cfg(all(test, not(any(feature = "client", feature = "facilitator"))))]
use keetanetwork_client as _;
#[cfg(all(test, not(any(feature = "client", feature = "facilitator"))))]
use keetanetwork_crypto as _;
#[cfg(not(any(feature = "client", feature = "facilitator")))]
use serde_json as _;
#[cfg(all(test, not(feature = "facilitator")))]
use tokio as _;
#[cfg(feature = "telemetry")]
use tracing_core as _;

pub mod chain;
pub mod exact;

mod networks;

pub use chain::DEFAULT_TOKEN_DECIMALS;
#[cfg(feature = "facilitator")]
pub use chain::KeetaChainProvider;
pub use exact::KeetaExact;
#[cfg(feature = "facilitator")]
pub use exact::KeetaExactFacilitator;
#[cfg(feature = "client")]
pub use exact::client::{KeetaExactClient, KeetaSigner};
pub use networks::*;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-keeta",
            "package name must match the crate directory"
        );
    }
}
