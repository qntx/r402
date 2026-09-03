//! Concordium chain support for the x402 payment protocol.
//!
//! Exact scheme: the buyer signs a V1 sponsored transaction (sender
//! signature only, empty sponsor slot). The facilitator adds its sponsor
//! signature, broadcasts via gRPC v2, and waits for `ConcordiumBFT`
//! finalization (~10s).
//!
//! Node access uses [`concordium_rust_sdk`] (MSRV 1.85, MPL-2.0). Default
//! gRPC endpoints match `@x402/concordium` 2.24:
//! `grpc.mainnet.concordium.software:20000` and
//! `grpc.testnet.concordium.com:20000`.
//!
//! Not on the umbrella `default` feature. Opt in with `r402/concordium`.
//!
//! # Features
//!
//! - `server` — [`ConcordiumExact::price_tag`]
//! - `client` — [`ConcordiumExactClient`]
//! - `facilitator` — in-process [`ConcordiumExactFacilitator::try_new`]
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
        reason = "unit tests panic on assertion failure; mock AFIT impls have no .await"
    )
)]

#[cfg(all(test, not(any(feature = "client", feature = "facilitator"))))]
use concordium_rust_sdk as _;
#[cfg(all(test, not(any(feature = "client", feature = "facilitator"))))]
use ed25519_dalek as _;
#[cfg(all(test, not(any(feature = "client", feature = "facilitator"))))]
use hex as _;
#[cfg(all(test, not(feature = "facilitator")))]
use tokio as _;
#[cfg(feature = "telemetry")]
use tracing_core as _;

pub mod chain;
pub mod exact;

mod networks;

#[cfg(feature = "facilitator")]
pub use chain::ConcordiumChainProvider;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::ConcordiumGrpc;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::ConcordiumSigner;
pub use chain::{
    CCD_ASSET_IDENTIFIER, CCD_DECIMALS, CONCORDIUM_ADDRESS_PATTERN, CONCORDIUM_MAINNET_CAIP2,
    CONCORDIUM_MAINNET_GRPC, CONCORDIUM_TESTNET_CAIP2, CONCORDIUM_TESTNET_GRPC,
    CONCORDIUM_WILDCARD_CAIP2, DEFAULT_FINALIZATION_TIMEOUT_MS, DEFAULT_GRPC_PORT,
    MAX_EXPIRY_OFFSET_SECONDS, USDR_TOKEN_ID, address_matches_regex, get_concordium_grpc_url,
    get_explorer_account_url, get_explorer_tx_url, parse_grpc_url,
};
pub use exact::ConcordiumExact;
#[cfg(feature = "facilitator")]
pub use exact::ConcordiumExactFacilitator;
#[cfg(feature = "client")]
pub use exact::client::ConcordiumExactClient;
pub use networks::*;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-concordium",
            "package name must match the crate directory"
        );
    }
}
