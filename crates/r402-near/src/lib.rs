//! NEAR chain support for the x402 payment protocol.
//!
//! Exact scheme: the buyer signs a NEP-366 `SignedDelegate` that authorizes
//! one NEP-141 `ft_transfer`. A facilitator-sponsored relayer submits the
//! outer transaction.
//!
//! # Features
//!
//! - `server` — [`NearExact::price_tag`]
//! - `client` — [`NearExactClient`] delegate signing
//! - `facilitator` — in-process [`NearExactFacilitator::try_new`]
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
#[cfg(not(any(feature = "client", feature = "facilitator", feature = "server")))]
use serde_json as _;
#[cfg(feature = "telemetry")]
use tracing_core as _;

#[cfg(test)]
mod unused_dev_deps {
    #[cfg(not(any(feature = "client", feature = "facilitator")))]
    use near_crypto as _;
    #[cfg(not(any(feature = "client", feature = "facilitator")))]
    use near_primitives as _;
    use tokio as _;
    #[cfg(not(any(feature = "client", feature = "facilitator")))]
    use wiremock as _;
}

pub mod chain;
pub mod exact;

mod networks;

#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::NearJsonRpc;
pub use chain::{
    DEFAULT_FT_TRANSFER_GAS, DEFAULT_MAX_SPONSORED_GAS, DEFAULT_TOKEN_DECIMALS,
    EMPTY_CONTRACT_CODE_HASH, ESTIMATED_BLOCK_SECONDS, FT_TRANSFER_METHOD, NONCE_RANGE_MULTIPLIER,
    ONE_YOCTO, timeout_blocks,
};
#[cfg(feature = "facilitator")]
pub use chain::{NearChainProvider, NearRelayer};
pub use exact::NearExact;
#[cfg(feature = "facilitator")]
pub use exact::NearExactFacilitator;
#[cfg(feature = "client")]
pub use exact::client::{NearExactClient, NearSigner};
pub use networks::*;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-near",
            "package name must match the crate directory"
        );
    }
}
