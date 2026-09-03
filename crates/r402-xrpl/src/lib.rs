//! XRPL chain support for the x402 payment protocol.
//!
//! Exact scheme: the buyer signs a `Payment` transaction and a facilitator
//! submits the signed blob. The payer pays the network fee; facilitators
//! do not sponsor fees.
//!
//! # Features
//!
//! - `server` — [`XrplExact::price_tag`]
//! - `client` — [`XrplExactClient`] payment signing
//! - `facilitator` — in-process [`XrplExactFacilitator::try_new`]
//! - `telemetry` — tracing spans on verify/settle

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    test,
    allow(
        unknown_lints,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::missing_assert_message,
        clippy::unused_async_trait_impl,
        reason = "unit tests panic on assertion failure; mock AFIT impls have no .await"
    )
)]

#[cfg(not(any(feature = "client", feature = "server", feature = "facilitator")))]
use serde_json as _;
#[cfg(feature = "telemetry")]
use tracing_core as _;

#[cfg(test)]
mod unused_dev_deps {
    #[cfg(not(any(feature = "client", feature = "facilitator")))]
    use hex as _;
    #[cfg(not(any(feature = "client", feature = "facilitator")))]
    use reqwest as _;
    #[cfg(not(any(feature = "client", feature = "facilitator")))]
    use sha2 as _;
    #[cfg(not(any(feature = "client", feature = "facilitator")))]
    use tokio as _;
    #[cfg(not(any(feature = "client", feature = "facilitator")))]
    use wiremock as _;
}

pub mod chain;
pub mod exact;

mod networks;

#[cfg(feature = "facilitator")]
pub use chain::XrplChainProvider;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::XrplJsonRpc;
pub use chain::{
    CANONICAL_SIGNING_PUB_KEY_LEN, DEFAULT_LEDGER_CLOSE_SECONDS, DEFAULT_LEDGER_TOLERANCE,
    DEFAULT_MAX_FEE_DROPS, LSF_DISABLE_MASTER, MAX_ACCOUNT_TICKETS, MAX_DESTINATION_TAG,
    RLUSD_CURRENCY, RLUSD_DECIMALS, RLUSD_MAINNET_ISSUER, RLUSD_TESTNET_ISSUER, SETTLEMENT_TTL_MS,
    TF_PARTIAL_PAYMENT, TRANSACTION_HASH_PREFIX, XRP_DECIMALS,
};
pub use exact::XrplExact;
#[cfg(feature = "facilitator")]
pub use exact::XrplExactFacilitator;
#[cfg(feature = "client")]
pub use exact::client::{XrplExactClient, XrplSigner};
pub use networks::*;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-xrpl",
            "package name must match the crate directory"
        );
    }
}
