//! Casper chain support for the x402 payment protocol.
//!
//! Exact scheme: CEP-18 `transfer_with_authorization` via a pre-signed
//! EIP-712 authorization. Cryptographic verification and on-chain
//! settlement are performed by a remote facilitator (default
//! `https://x402-facilitator.cspr.cloud`).
//!
//! # Features
//!
//! - `server` — [`CasperExact::price_tag`]
//! - `client` — [`CasperExactClient`] EIP-712 signing
//! - `facilitator` — remote [`CasperExactFacilitator::try_new`]
//! - `telemetry` — tracing spans on verify/settle/supported

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

#[cfg(all(test, not(feature = "client")))]
use r402_client as _;
#[cfg(all(test, not(feature = "facilitator")))]
use r402_facilitator as _;
#[cfg(all(test, not(feature = "server")))]
use r402_server as _;
#[cfg(all(test, not(feature = "client")))]
use rand as _;
#[cfg(all(test, not(feature = "facilitator")))]
use reqwest as _;
#[cfg(not(any(feature = "client", feature = "server", feature = "facilitator")))]
use serde_json as _;
#[cfg(all(test, not(feature = "client")))]
use sha3 as _;
#[cfg(all(test, not(any(feature = "client", feature = "facilitator"))))]
use tokio as _;
#[cfg(all(feature = "telemetry", not(feature = "facilitator")))]
use tracing as _;
#[cfg(feature = "telemetry")]
use tracing_core as _;
#[cfg(all(test, not(feature = "facilitator")))]
use url as _;

pub mod chain;
pub mod exact;

mod networks;

pub use chain::{CSPR_DECIMALS, MOTES_PER_CSPR, Motes, MotesParseError};
pub use exact::CasperExact;
#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub use exact::client::{AuthorizationBuilder, CasperExactClient, CasperSigner};
#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub use exact::eip712::Eip712Domain;
#[cfg(feature = "facilitator")]
#[cfg_attr(docsrs, doc(cfg(feature = "facilitator")))]
pub use exact::facilitator::{
    CASPER_DOCS_URL, CasperExactFacilitator, CasperFacilitatorConfig, FACILITATOR_URL_ENV,
    ReqwestTransport,
};
pub use networks::*;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-casper",
            "package name must match the crate directory"
        );
    }
}
