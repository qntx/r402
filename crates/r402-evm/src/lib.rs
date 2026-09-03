//! EIP-155 (EVM) chain support for the x402 payment protocol.
//!
//! Exact scheme: ERC-3009 `transferWithAuthorization` and Permit2 via
//! `x402ExactPermit2Proxy`. Upto scheme: Permit2 witness via
//! `x402UptoPermit2Proxy` (verify = max, settle = actual ≤ max).
//! EIP-1271 / EIP-6492 smart-wallet signatures are accepted on verify
//! and settle. Insufficient Permit2 allowance is
//! [`r402_protocol::error::VerificationError::Permit2AllowanceRequired`]
//! (HTTP 412).
//!
//! # Features
//!
//! - `server` — [`Eip155Exact::price_tag`], [`Eip155Upto::price_tag`]
//! - `client` — [`Eip155ExactClient`] / [`Eip155UptoClient`] EIP-712 signing
//! - `facilitator` — [`Eip155ExactFacilitator::try_new`], [`Eip155UptoFacilitator::try_new`]
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
pub mod eip2612;
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
pub mod upto;

pub use asset::{AssetTransferMethod, VALIDATOR_ADDRESS};
pub use chain::EVM_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS;
pub use eip2612::{
    EIP2612_GAS_SPONSORING_KEY, EIP2612_GAS_SPONSORING_VERSION, Eip2612ParseError,
    Eip2612SignedPermit,
};
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
#[cfg(feature = "facilitator")]
pub use upto::Eip155UptoFacilitator;
pub use upto::{Eip155Upto, X402_UPTO_PERMIT2_PROXY};
#[cfg(feature = "client")]
pub use upto::{
    Eip155UptoClient, Eip155UptoClientBuilder, Eip2612SigningParams, sign_eip2612_permit,
};

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
