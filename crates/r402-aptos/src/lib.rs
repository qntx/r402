//! Aptos chain support for the x402 payment protocol.
//!
//! Exact scheme: the buyer signs a `0x1::primary_fungible_store::transfer`
//! of a fungible asset. The facilitator is the optional `feePayer`: it
//! verifies the payload, adds its signature, and submits via `aptos-sdk`.
//!
//! # Features
//!
//! - `server` — [`AptosExact::price_tag`]
//! - `client` — [`AptosExactClient`] transfer signing
//! - `facilitator` — in-process [`AptosExactFacilitator::try_new`]
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

#[cfg(all(test, not(any(feature = "client", feature = "facilitator"))))]
use aptos_sdk as _;
#[cfg(not(any(feature = "client", feature = "facilitator")))]
use base64 as _;
#[cfg(not(any(feature = "server", feature = "facilitator", feature = "client")))]
use serde_json as _;
#[cfg(test)]
use tokio as _;
#[cfg(feature = "telemetry")]
use tracing_core as _;

pub mod chain;
pub mod exact;

mod networks;

#[cfg(feature = "facilitator")]
pub use chain::{AptosChainProvider, AptosFeePayer};
pub use chain::{
    DEFAULT_CLIENT_MAX_GAS, DEFAULT_GAS_UNIT_PRICE, DEFAULT_TOKEN_DECIMALS,
    EXPIRATION_BUFFER_SECONDS, FUNGIBLE_ASSET_METADATA, FUNGIBLE_ASSET_TRANSFER, MAX_GAS_AMOUNT,
    MAX_GAS_UNIT_PRICE, PRIMARY_FUNGIBLE_STORE_TRANSFER, USDC_MAINNET_FA, USDC_TESTNET_FA,
};
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::{
    DecodedAptosPayload, SimpleTransaction, decode_aptos_payload, encode_aptos_payload,
};
pub use exact::AptosExact;
#[cfg(feature = "facilitator")]
pub use exact::AptosExactFacilitator;
#[cfg(feature = "client")]
pub use exact::client::{AptosExactClient, AptosSigner};
pub use networks::*;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-aptos",
            "package name must match the crate directory"
        );
    }
}
