//! EIP-2612 gas-sponsoring extension wire types (exact + upto).
//!
//! Official key: `eip2612GasSponsoring`. Client `info` fields match
//! `specs/extensions/eip2612_gas_sponsoring.md`:
//! `from`, `asset`, `spender`, `amount`, `nonce`, `deadline`, `signature`, `version`.
//!
//! Facilitators consume this for `settleWithPermit` on both exact and upto
//! Permit2 proxies.

use alloy_primitives::{Address, Bytes};
use r402_core::wire::{ExtensionEntry, Extensions};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::chain::TokenAmount;

/// Stable extension identifier on the wire.
pub const EIP2612_GAS_SPONSORING_KEY: &str = "eip2612GasSponsoring";

/// Schema version written into client-populated `info.version`.
pub const EIP2612_GAS_SPONSORING_VERSION: &str = "1";

/// Buyer-signed EIP-2612 permit for gasless Permit2 allowance.
///
/// Wire field names are official camelCase (`amount`, not legacy `value`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Eip2612SignedPermit {
    /// Permit owner (== payment payer).
    pub from: Address,
    /// ERC-20 token contract.
    pub asset: Address,
    /// Spender — MUST be the canonical Permit2 address.
    pub spender: Address,
    /// Approved allowance (uint256 string on the wire via [`TokenAmount`]).
    pub amount: TokenAmount,
    /// EIP-2612 nonce of the owner at sign time.
    pub nonce: TokenAmount,
    /// Permit deadline (Unix seconds).
    pub deadline: TokenAmount,
    /// 65-byte packed `(r, s, v)` EOA signature.
    pub signature: Bytes,
    /// Schema version (`"1"`).
    pub version: String,
}

/// Errors raised while extracting [`Eip2612SignedPermit`] from extensions.
#[derive(Debug, Error)]
pub enum Eip2612ParseError {
    /// Extension JSON did not match the expected shape.
    #[error("invalid eip2612GasSponsoring extension payload: {0}")]
    Invalid(#[from] serde_json::Error),
    /// Signature was not 65 bytes.
    #[error("eip2612 signature MUST be 65 bytes long, got {0}")]
    InvalidSignatureLength(usize),
}

impl Eip2612SignedPermit {
    /// Length of the canonical packed `(r, s, v)` signature in bytes.
    pub const SIGNATURE_LEN: usize = 65;

    /// Decodes the extension from a payload extensions map.
    ///
    /// # Errors
    ///
    /// Malformed JSON or wrong signature length.
    pub fn from_extensions(extensions: &Extensions) -> Result<Option<Self>, Eip2612ParseError> {
        let Some(entry) = extensions.get(EIP2612_GAS_SPONSORING_KEY) else {
            return Ok(None);
        };
        let parsed: Self = match entry {
            ExtensionEntry::Structured { info, .. } => serde_json::from_value(info.clone())?,
            ExtensionEntry::Raw(value) => serde_json::from_value(value.clone())?,
        };
        if parsed.signature.len() != Self::SIGNATURE_LEN {
            return Err(Eip2612ParseError::InvalidSignatureLength(
                parsed.signature.len(),
            ));
        }
        Ok(Some(parsed))
    }

    /// Splits the packed signature into `(r, s, v)`.
    #[must_use]
    pub fn split_signature(&self) -> Option<(alloy_primitives::B256, alloy_primitives::B256, u8)> {
        let bytes = self.signature.as_ref();
        if bytes.len() != Self::SIGNATURE_LEN {
            return None;
        }
        let mut r = [0u8; 32];
        let mut s = [0u8; 32];
        r.copy_from_slice(bytes.get(0..32)?);
        s.copy_from_slice(bytes.get(32..64)?);
        let v = *bytes.get(64)?;
        Some((
            alloy_primitives::B256::from(r),
            alloy_primitives::B256::from(s),
            v,
        ))
    }

    /// Builds a structured extension entry for payload insertion.
    ///
    /// # Errors
    ///
    /// Serde failures (should not occur for this type).
    pub fn to_extension_entry(&self) -> Result<ExtensionEntry, serde_json::Error> {
        Ok(ExtensionEntry::info(serde_json::to_value(self)?))
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, Bytes, U256};
    use r402_core::wire::{ExtensionEntry, Extensions};
    use serde_json::json;

    use super::*;

    fn sample_permit() -> Eip2612SignedPermit {
        Eip2612SignedPermit {
            from: Address::repeat_byte(0xAA),
            asset: Address::repeat_byte(0xBB),
            spender: Address::repeat_byte(0xCC),
            amount: TokenAmount::from(U256::from(1_000_000_u64)),
            nonce: TokenAmount::from(U256::from(7_u64)),
            deadline: TokenAmount::from(U256::from(1_700_000_000_u64)),
            signature: Bytes::from(vec![0x42u8; 65]),
            version: EIP2612_GAS_SPONSORING_VERSION.into(),
        }
    }

    #[test]
    fn wire_uses_amount_not_value() {
        let v = serde_json::to_value(sample_permit()).unwrap();
        assert!(v.get("amount").is_some());
        assert!(v.get("value").is_none());
        assert!(v.get("nonce").is_some());
        assert_eq!(v.get("version"), Some(&serde_json::json!("1")));
    }

    #[test]
    fn extension_round_trips_structured() {
        let permit = sample_permit();
        let mut extensions = Extensions::new();
        extensions.insert(
            EIP2612_GAS_SPONSORING_KEY,
            permit.to_extension_entry().unwrap(),
        );
        let extracted = Eip2612SignedPermit::from_extensions(&extensions)
            .unwrap()
            .unwrap();
        assert_eq!(extracted, permit);
    }

    #[test]
    fn missing_extension_ok_none() {
        assert!(
            Eip2612SignedPermit::from_extensions(&Extensions::new())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn short_signature_rejected() {
        let mut permit = sample_permit();
        permit.signature = Bytes::from(vec![0u8; 64]);
        let mut extensions = Extensions::new();
        extensions.insert(
            EIP2612_GAS_SPONSORING_KEY,
            permit.to_extension_entry().unwrap(),
        );
        assert!(matches!(
            Eip2612SignedPermit::from_extensions(&extensions),
            Err(Eip2612ParseError::InvalidSignatureLength(64))
        ));
    }

    #[test]
    fn malformed_rejected() {
        let mut extensions = Extensions::new();
        extensions.insert(
            EIP2612_GAS_SPONSORING_KEY,
            ExtensionEntry::raw(json!({ "from": "not-an-address" })),
        );
        assert!(matches!(
            Eip2612SignedPermit::from_extensions(&extensions),
            Err(Eip2612ParseError::Invalid(_))
        ));
    }
}
