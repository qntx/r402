//! XRPL chain support for x402 payments.
//!
//! - [`XrplChainReference`] — CAIP-2 numeric `NetworkID` (`0` / `1` / `2`)
//! - [`XrplClassicAddress`] — classic `r-address`
//! - [`XrplTokenDeployment`] — XRP drops or issued-currency metadata
//! - [`XrplChainProvider`] — facilitator JSON-RPC (feature `facilitator`)

/// Core XRPL chain types (addresses, references, token deployments).
pub mod types;
pub use types::*;

/// Binary codec, signing, and decimal helpers.
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod codec;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use codec::*;

/// JSON-RPC client abstraction for XRPL.
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod rpc;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use rpc::*;

/// XRPL chain provider implementation.
#[cfg(feature = "facilitator")]
pub mod provider;
#[cfg(feature = "facilitator")]
pub use provider::*;
