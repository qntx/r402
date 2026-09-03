//! Tron "exact" payment scheme implementation.
//!
//! Reuses EIP-3009 `transferWithAuthorization` and Permit2. TIP-712 is
//! byte-compatible with EIP-712. Differences from `r402-evm`:
//!
//! - **EOA-only signatures**: no EIP-1271 / EIP-6492 (Tron has no contract wallets).
//! - **`Base58Check` wire addresses**: converted to raw EVM hex only at the
//!   TIP-712 signing boundary.

use r402_protocol::scheme::SchemeId;

#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub mod client;

pub mod error;
pub use error::*;

#[cfg(feature = "facilitator")]
#[cfg_attr(docsrs, doc(cfg(feature = "facilitator")))]
pub mod facilitator;
#[cfg(feature = "facilitator")]
pub use facilitator::TronExactFacilitator;

pub mod payload;
pub use payload::*;

#[cfg(feature = "server")]
#[cfg_attr(docsrs, doc(cfg(feature = "server")))]
pub mod server;

/// Tron exact scheme identifier.
///
/// Uses CAIP-2 chain IDs (e.g. `tron:0x2b6653dc`) for chain identification
/// and embeds requirements directly in the payload.
#[derive(Debug, Clone, Copy)]
pub struct TronExact;

impl SchemeId for TronExact {
    fn namespace(&self) -> &'static str {
        "tron"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}
