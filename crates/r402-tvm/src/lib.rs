//! TON (TVM) chain support for the x402 payment protocol.
//!
//! Exact scheme: the buyer signs a W5R1 `internal_signed` message that
//! authorizes one TEP-74 `jetton_transfer`, and a facilitator Highload V3
//! wallet relays it while sponsoring gas.
//!
//! # Features
//!
//! - `server` — [`TvmExact::price_tag`]
//! - `client` — [`TvmExactClient`] W5R1 settlement signing
//! - `facilitator` — in-process [`TvmExactFacilitator::try_new`]
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
#![allow(
    clippy::doc_markdown,
    reason = "TON terms (W5R1, TEP-74, Highload, BoC, jetton) are not intra-doc links"
)]
#![allow(
    unexpected_cfgs,
    reason = "compile_error guard for forbidden tonlib-sys / tonlib-client"
)]

#[cfg(any(feature = "tonlib-sys", feature = "tonlib-client"))]
compile_error!("r402-tvm must never depend on tonlib-sys or tonlib-client");

#[cfg(not(any(feature = "client", feature = "facilitator")))]
use base64 as _;
#[cfg(not(any(feature = "client", feature = "server", feature = "facilitator", test)))]
use serde_json as _;
#[cfg(feature = "telemetry")]
use tracing_core as _;

pub mod chain;
pub mod exact;

mod networks;

#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::TvmRpc;
pub use chain::{
    DEFAULT_HIGHLOAD_SUBWALLET_ID, DEFAULT_HIGHLOAD_TIMEOUT, DEFAULT_JETTON_WALLET_MESSAGE_AMOUNT,
    DEFAULT_MAX_TIMEOUT_SECONDS, DEFAULT_RELAY_AMOUNT,
    DEFAULT_SETTLEMENT_BATCH_FLUSH_INTERVAL_SECONDS, DEFAULT_SETTLEMENT_BATCH_FLUSH_SIZE,
    DEFAULT_SETTLEMENT_BATCH_MAX_SIZE, DEFAULT_TOKEN_DECIMALS,
    DEFAULT_TONCENTER_EMULATION_TIMEOUT_SECONDS, DEFAULT_TONCENTER_TIMEOUT_SECONDS,
    DEFAULT_TRACE_CONFIRMATION_TIMEOUT_SECONDS, DEFAULT_TVM_EMULATION_ADDRESS,
    DEFAULT_TVM_EMULATION_RELAY_AMOUNT, DEFAULT_TVM_EMULATION_SEQNO,
    DEFAULT_TVM_EMULATION_WALLET_ID, DEFAULT_TVM_INNER_GAS_BUFFER, DEFAULT_TVM_OUTER_GAS_BUFFER,
    DEFAULT_VALID_UNTIL_OFFSET, DEFAULT_W5R1_SUBWALLET_NUMBER, EXTERNAL_SIGNED_OP,
    HIGHLOAD_V3_CODE_HASH, HIGHLOAD_V3_CODE_HEX, INTERNAL_SIGNED_OP, JETTON_TRANSFER_OP,
    MIN_FACILITATOR_TON_BALANCE, SEND_MODE_IGNORE_ERRORS, SEND_MODE_PAY_FEES_SEPARATELY,
    SEND_MSG_OP, TONAPI_MAINNET_BASE_URL, TONAPI_TESTNET_BASE_URL, TONCENTER_MAINNET_BASE_URL,
    TONCENTER_TESTNET_BASE_URL, USDT_MAINNET_MINTER, USDT_TESTNET_MINTER, W5R1_CODE_HASH,
    W5R1_CODE_HEX,
};
#[cfg(feature = "facilitator")]
pub use chain::{HighloadV3Config, TvmChainProvider};
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::{TvmRestClient, default_base_url};
pub use exact::TvmExact;
#[cfg(feature = "facilitator")]
pub use exact::TvmExactFacilitator;
#[cfg(feature = "client")]
pub use exact::client::{TvmExactClient, TvmW5Signer};
pub use networks::*;

#[cfg(test)]
mod unused_dev_deps {
    #[cfg(not(any(feature = "client", feature = "facilitator")))]
    use ed25519_dalek as _;
    #[cfg(not(feature = "facilitator"))]
    use r402_facilitator as _;
    use tokio as _;
}

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-tvm",
            "package name must match the crate directory"
        );
    }
}
