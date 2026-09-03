//! Hedera chain support for the x402 payment protocol.
//!
//! Exact scheme: a payer-signed `TransferTransaction` that credits `payTo`.
//! The facilitator is the fee payer: it verifies via Mirror Node, adds its
//! signature, and submits. `aliasPolicy` is facilitator config (default
//! reject), not wire extra.
//!
//! # Features
//!
//! - `server` — [`HederaExact::price_tag`]
//! - `client` — [`HederaExactClient`] transfer construction
//! - `facilitator` — in-process [`HederaExactFacilitator::try_new`]
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

#[cfg(not(any(feature = "client", feature = "facilitator")))]
use base64 as _;
#[cfg(not(any(feature = "client", feature = "facilitator")))]
use serde_json as _;
#[cfg(feature = "telemetry")]
use tracing_core as _;

#[cfg(test)]
mod unused_dev_deps {
    #[cfg(not(feature = "facilitator"))]
    use reqwest as _;
    #[cfg(not(any(feature = "client", feature = "facilitator")))]
    use tokio as _;
    #[cfg(not(feature = "facilitator"))]
    use wiremock as _;
}

pub mod chain;
pub mod exact;

mod networks;

#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::inspect_hedera_transaction;
#[cfg(feature = "facilitator")]
pub use chain::{HederaChainProvider, HederaFeePayer, HederaMirrorClient};
pub use exact::HederaExact;
#[cfg(feature = "client")]
pub use exact::client::{HederaExactClient, HederaSigner};
#[cfg(feature = "facilitator")]
pub use exact::facilitator::{AliasPolicy, HederaExactFacilitator, HederaExactFacilitatorConfig};
pub use networks::*;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-hedera",
            "package name must match the crate directory"
        );
    }
}
