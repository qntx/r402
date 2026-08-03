//! Casper "exact" payment scheme implementation.
//!
//! This module implements the "exact" payment scheme for the Casper Network
//! using a CEP-18 token's `transfer_with_authorization` entry point: the
//! buyer signs an EIP-712 `TransferWithAuthorization` message and the
//! facilitator relays it on-chain, paying the gas.
//!
//! # Features
//!
//! - CEP-18 (wCSPR) settlement with exact 9-decimal mote arithmetic
//! - Tagged Casper address and public-key validation (ed25519 / secp256k1)
//! - Local pre-flight validation before any network round trip
//! - Facilitator client for `/verify`, `/settle`, and `/supported`
//!
//! # Payload Structure
//!
//! ```json
//! {
//!   "signature": "<130 hex chars>",
//!   "publicKey": "01<64 hex chars>",
//!   "authorization": {
//!     "from": "00<64 hex chars>",
//!     "to": "00<64 hex chars>",
//!     "value": "1500000000",
//!     "validAfter": "1700000000",
//!     "validBefore": "1700000600",
//!     "nonce": "<64 hex chars>"
//!   }
//! }
//! ```

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

pub mod verify;
pub use verify::{ValidatedPayment, validate_at, validate_payload_shape, validate_request};

/// Casper exact scheme identifier.
///
/// Uses CAIP-2 chain IDs (`casper:casper`, `casper:casper-test`) for chain
/// identification and embeds requirements directly in the payload.
#[derive(Debug, Clone, Copy)]
pub struct CasperExact;

impl SchemeId for CasperExact {
    fn namespace(&self) -> &'static str {
        "casper"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_identity_matches_caip2_family() {
        assert_eq!(CasperExact.namespace(), "casper");
        assert_eq!(CasperExact.scheme(), "exact");
        assert_eq!(CasperExact.caip_family(), "casper:*");
        assert_eq!(CasperExact.id(), "casper-exact");
    }
}
