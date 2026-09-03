//! Algorand chain support for the x402 payment protocol.
//!
//! Exact scheme: an atomic group with an ASA transfer to `payTo` and an
//! optional 0-amount fee-payer payment. The facilitator signs the fee-payer
//! transaction, simulates, broadcasts, and waits for confirmation.
//!
//! # Features
//!
//! - `server` — [`AlgorandExact::price_tag`]
//! - `client` — [`AlgorandExactClient`] atomic-group construction
//! - `facilitator` — in-process [`AlgorandExactFacilitator::try_new`]
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
use ed25519_dalek as _;
#[cfg(not(any(feature = "server", feature = "facilitator")))]
use serde_json as _;
#[cfg(all(test, not(feature = "facilitator")))]
use tokio as _;
#[cfg(feature = "telemetry")]
use tracing_core as _;
#[cfg(all(test, not(any(feature = "client", feature = "facilitator"))))]
use wiremock as _;

pub mod chain;
pub mod exact;

mod networks;

#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::AlgodClient;
#[cfg(feature = "facilitator")]
pub use chain::AlgorandChainProvider;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::AlgorandSigner;
pub use chain::{
    DEFAULT_TOKEN_DECIMALS, MAX_REASONABLE_FEE_PER_TXN, MAX_TRANSACTION_GROUP_SIZE,
    max_reasonable_group_fee,
};
#[cfg(feature = "facilitator")]
pub use exact::AlgorandExactFacilitator;
#[cfg(feature = "client")]
pub use exact::client::AlgorandExactClient;
pub use exact::{AlgorandExact, DEFAULT_VALIDITY_ROUNDS, DEFAULT_WAIT_ROUNDS};
pub use networks::*;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-algorand",
            "package name must match the crate directory"
        );
    }
}
