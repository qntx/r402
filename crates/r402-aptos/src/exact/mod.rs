//! Aptos `"exact"` payment scheme implementation.
//!
//! The buyer signs a `primary_fungible_store::transfer`. The facilitator
//! verifies that payload against `PaymentRequirements` and submits it,
//! adding a fee-payer signature when `extra.feePayer` is set.

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

/// Aptos exact scheme identifier.
///
/// Uses CAIP-2 chain IDs (`aptos:1`, `aptos:2`) and embeds
/// requirements directly in the payload.
#[derive(Debug, Clone, Copy)]
pub struct AptosExact;

impl SchemeId for AptosExact {
    fn namespace(&self) -> &'static str {
        "aptos"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}
