//! Hedera `"exact"` payment scheme implementation.
//!
//! The buyer signs a `TransferTransaction` that credits `payTo` for `amount`
//! of the required asset. The facilitator is the fee payer: it verifies the
//! payload, adds its signature, and submits.

use r402_protocol::scheme::SchemeId;

#[cfg(feature = "client")]
pub mod client;

pub mod error;
pub use error::*;

#[cfg(feature = "facilitator")]
pub mod facilitator;
#[cfg(feature = "facilitator")]
pub use facilitator::HederaExactFacilitator;

pub mod payload;
pub use payload::*;

#[cfg(feature = "server")]
pub mod server;

/// Hedera exact scheme identifier.
///
/// Uses CAIP-2 chain IDs (`hedera:mainnet`, `hedera:testnet`) and embeds
/// requirements directly in the payload.
#[derive(Debug, Clone, Copy)]
pub struct HederaExact;

impl SchemeId for HederaExact {
    fn namespace(&self) -> &'static str {
        "hedera"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}
