//! XRPL `"exact"` payment scheme implementation.
//!
//! The buyer signs a `Payment` transaction. The facilitator verifies that
//! payload against `PaymentRequirements` and submits the signed blob.

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

/// XRPL exact scheme identifier.
///
/// Uses CAIP-2 chain IDs (`xrpl:0`, `xrpl:1`, `xrpl:2`) and embeds
/// requirements directly in the payload.
#[derive(Debug, Clone, Copy)]
pub struct XrplExact;

impl SchemeId for XrplExact {
    fn namespace(&self) -> &'static str {
        "xrpl"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}
