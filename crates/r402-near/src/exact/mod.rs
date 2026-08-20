//! NEAR `"exact"` payment scheme implementation.
//!
//! The buyer signs a NEP-366 `SignedDelegate` that authorizes one NEP-141
//! `ft_transfer`. The facilitator verifies that payload against
//! `PaymentRequirements` and submits it through a local relayer.

use r402_core::scheme::SchemeId;

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "facilitator")]
pub mod facilitator;

#[cfg(feature = "client")]
pub mod client;

pub mod error;
pub use error::*;

pub mod types;
pub use types::*;

/// NEAR exact scheme identifier.
///
/// Uses CAIP-2 chain IDs (`near:mainnet`, `near:testnet`) and embeds
/// requirements directly in the payload.
#[derive(Debug, Clone, Copy)]
pub struct NearExact;

impl SchemeId for NearExact {
    fn namespace(&self) -> &'static str {
        "near"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}
