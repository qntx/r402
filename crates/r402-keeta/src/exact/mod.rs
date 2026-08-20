//! Keeta `"exact"` payment scheme implementation.
//!
//! The buyer signs a block containing one `SEND`. The facilitator verifies
//! that payload against `PaymentRequirements` and transmits it with a
//! fee-payer signature.

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

/// Keeta exact scheme identifier.
///
/// Uses CAIP-2 chain IDs (`keeta:21378`, `keeta:1413829460`) and embeds
/// requirements directly in the payload.
#[derive(Debug, Clone, Copy)]
pub struct KeetaExact;

impl SchemeId for KeetaExact {
    fn namespace(&self) -> &'static str {
        "keeta"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}
