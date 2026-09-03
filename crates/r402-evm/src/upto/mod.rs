//! EIP-155 "upto" payment scheme (Permit2 witness, usage-based).
//!
//! Buyer signs a maximum; the facilitator settles any amount in `[0, max]`.
//! Verify amount = signed max; settle amount = actual ≤ max (HTTP Sequential
//! patches `paymentRequirements.amount` via `UptoActualAmount`).
//!
//! Transfer method is Permit2 only. EIP-3009 requires an exact amount at
//! signature time and cannot express a ceiling.

use alloy_primitives::{Address, address};
use r402_protocol::scheme::SchemeId;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "facilitator")]
pub mod facilitator;
pub mod payload;
#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "client")]
pub use client::{
    Eip155UptoClient, Eip155UptoClientBuilder, Eip2612SigningParams, sign_eip2612_permit,
};
#[cfg(feature = "facilitator")]
pub use facilitator::Eip155UptoFacilitator;
pub use payload::*;

pub use crate::eip2612::{EIP2612_GAS_SPONSORING_KEY, Eip2612ParseError, Eip2612SignedPermit};

/// Canonical x402 upto Permit2 proxy. Same CREATE2 address on every chain.
///
/// `settle` takes a runtime `amount` constrained by `amount <= permit.permitted.amount`.
pub const X402_UPTO_PERMIT2_PROXY: Address = address!("0x4020A4f3b7b90ccA423B9fabCc0CE57C6C240002");

/// EIP-155 upto payment scheme identifier.
#[derive(Debug, Clone, Copy)]
pub struct Eip155Upto;

impl SchemeId for Eip155Upto {
    fn namespace(&self) -> &'static str {
        "eip155"
    }

    fn scheme(&self) -> &str {
        UptoScheme.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_id_namespace_and_name() {
        assert_eq!(Eip155Upto.namespace(), "eip155");
        assert_eq!(Eip155Upto.scheme(), "upto");
    }

    #[test]
    fn proxy_address_is_canonical_x402_upto() {
        assert_eq!(
            X402_UPTO_PERMIT2_PROXY.to_checksum(None),
            "0x4020A4f3b7b90ccA423B9fabCc0CE57C6C240002"
        );
    }
}
