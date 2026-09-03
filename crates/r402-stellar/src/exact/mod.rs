//! Stellar `"exact"` payment scheme implementation.
//!
//! The buyer signs Soroban authorization entries that authorize one SEP-41
//! `transfer`. The facilitator verifies that payload against
//! `PaymentRequirements` and submits it as the transaction source.

use r402_protocol::scheme::SchemeId;

#[cfg(feature = "client")]
pub mod client;

pub mod error;
pub use error::*;

#[cfg(feature = "facilitator")]
pub mod facilitator;
#[cfg(feature = "facilitator")]
pub use facilitator::StellarExactFacilitator;

pub mod payload;
pub use payload::*;

#[cfg(feature = "server")]
pub mod server;

/// Stellar exact scheme identifier.
///
/// Uses CAIP-2 chain IDs (`stellar:pubnet`, `stellar:testnet`) and embeds
/// requirements directly in the payload.
#[derive(Debug, Clone, Copy)]
pub struct StellarExact;

impl SchemeId for StellarExact {
    fn namespace(&self) -> &'static str {
        "stellar"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}
