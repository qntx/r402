//! Tron chain support. **Experimental / not an x402 protocol mechanism.**
//!
//! Exact scheme: EIP-3009 `transferWithAuthorization` and SUN.io Permit2 via
//! `x402ExactPermit2Proxy`. Signatures are EOA-only (no EIP-1271 / EIP-6492).
//! Wire addresses are `Base58Check`; TIP-712 uses the raw 20-byte EVM form.
//! Not re-exported from umbrella `r402` (no `tron` feature).
//!
//! # Features
//!
//! - `server` — [`TronExact::price_tag`]
//! - `client` — [`TronExactClient`] TIP-712 signing
//! - `facilitator` — in-process [`TronExactFacilitator::try_new`]
//! - `telemetry` — tracing spans on `TronGrid` paths

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

#[cfg(not(feature = "facilitator"))]
use compact_str as _;
#[cfg(feature = "telemetry")]
use tracing_core as _;

#[cfg(test)]
mod unused_dev_deps {
    #[cfg(not(any(feature = "client", feature = "facilitator")))]
    use alloy_signer_local as _;
    #[cfg(not(feature = "facilitator"))]
    use tokio as _;
    #[cfg(not(feature = "facilitator"))]
    use url as _;
    #[cfg(not(feature = "facilitator"))]
    use wiremock as _;
}

pub mod chain;
pub mod exact;

mod networks;

pub use chain::TRON_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS;
#[cfg(feature = "facilitator")]
#[cfg_attr(docsrs, doc(cfg(feature = "facilitator")))]
pub use exact::TronExactFacilitator;
#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub use exact::client::{Eip3009SigningParams, Permit2SigningParams, SignerLike, TronExactClient};
pub use exact::{AssetTransferMethod, TronExact};
pub use networks::*;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-tron",
            "package name must match the crate directory"
        );
    }
}
