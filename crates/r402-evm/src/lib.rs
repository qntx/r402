//! EIP-155 (EVM) chain support for the x402 payment protocol.
//!
//! Exact scheme: ERC-3009 `transferWithAuthorization` and Permit2 via
//! `x402ExactPermit2Proxy`. EIP-1271 / EIP-6492 smart-wallet signatures
//! are accepted on verify and settle.
//!
//! # Features
//!
//! - `server` — [`Eip155Exact::price_tag`]
//! - `client` — [`Eip155ExactClient`] EIP-712 signing
//! - `facilitator` — in-process [`Eip155ExactFacilitator::try_new`]
//! - `telemetry` — tracing spans on RPC paths

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    test,
    allow(
        clippy::indexing_slicing,
        clippy::panic,
        reason = "unit tests panic on assertion failure"
    )
)]

// Consumed by feature-gated modules; silence the linter when default
// features disable every consumer of `compact_str`.
#[cfg(not(any(feature = "client", feature = "server", feature = "facilitator")))]
use compact_str as _;

pub mod asset;
pub mod chain;
#[cfg(feature = "facilitator")]
#[cfg_attr(docsrs, doc(cfg(feature = "facilitator")))]
pub mod error;
pub mod exact;
mod networks;
pub mod permit2;
#[cfg(feature = "facilitator")]
#[cfg_attr(docsrs, doc(cfg(feature = "facilitator")))]
pub mod signature;
#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub mod signer;

pub use asset::{AssetTransferMethod, VALIDATOR_ADDRESS};
pub use chain::EVM_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS;
#[cfg(feature = "facilitator")]
pub use error::Eip155ExactError;
pub use exact::Eip155Exact;
#[cfg(feature = "facilitator")]
pub use exact::Eip155ExactFacilitator;
#[cfg(feature = "client")]
pub use exact::{Eip155ExactClient, Eip155ExactClientBuilder};
pub use networks::*;
pub use permit2::{PERMIT2_ADDRESS, Permit2TokenPermissions};
#[cfg(feature = "client")]
pub use permit2::{Permit2Approver, permit2_allowance_calldata, permit2_approval_calldata};
#[cfg(feature = "facilitator")]
pub use signature::StructuredSignatureFormatError;
#[cfg(feature = "client")]
pub use signer::SignerLike;

#[cfg(test)]
mod _dev_deps {
    #[cfg(not(feature = "facilitator"))]
    use alloy_network as _;
    #[cfg(not(any(feature = "client-provider", feature = "facilitator")))]
    use alloy_provider as _;
    #[cfg(not(feature = "client"))]
    use alloy_signer_local as _;
    #[cfg(not(feature = "facilitator"))]
    use alloy_transport_http as _;
    use tokio as _;
    #[cfg(not(feature = "facilitator"))]
    use url as _;
}

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-evm",
            "package name must match the crate directory"
        );
    }
}
