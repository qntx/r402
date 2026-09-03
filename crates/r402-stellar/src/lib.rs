//! Stellar chain support for the x402 payment protocol.
//!
//! Exact scheme: the buyer signs Soroban authorization entries for one
//! SEP-41 `transfer`. The facilitator is the transaction source / fee sponsor.
//!
//! # Features
//!
//! - `server` — [`StellarExact::price_tag`]
//! - `client` — [`StellarExactClient`] authorization-entry signing
//! - `facilitator` — in-process [`StellarExactFacilitator::try_new`]
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
    use ed25519_dalek as _;
    #[cfg(not(any(feature = "client", feature = "facilitator")))]
    use sha2 as _;
    #[cfg(not(any(feature = "client", feature = "facilitator")))]
    use stellar_xdr as _;
    #[cfg(not(any(feature = "client", feature = "facilitator")))]
    use tokio as _;
    #[cfg(not(any(feature = "client", feature = "facilitator")))]
    use wiremock as _;
}

pub mod chain;
pub mod exact;

mod networks;

#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::StellarJsonRpc;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::network_id;
pub use chain::{
    BASE_FEE_STROOPS, DEFAULT_ESTIMATED_LEDGER_SECONDS, DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
    DEFAULT_TIMEOUT_SECONDS, DEFAULT_TOKEN_DECIMALS, HORIZON_LEDGERS_SAMPLE_SIZE, NULL_ACCOUNT,
    SIGNATURE_EXPIRATION_LEDGER_TOLERANCE, TRANSFER_FUNCTION, timeout_ledgers,
};
#[cfg(feature = "facilitator")]
pub use chain::{StellarChainProvider, StellarFacilitatorError};
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::{StellarRpc, StellarRpcError, StellarSigner};
pub use exact::StellarExact;
#[cfg(feature = "facilitator")]
pub use exact::StellarExactFacilitator;
#[cfg(feature = "client")]
pub use exact::client::StellarExactClient;
pub use networks::*;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-stellar",
            "package name must match the crate directory"
        );
    }
}
