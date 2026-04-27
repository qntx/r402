//! EIP-2612 gas-sponsoring extension for the upto scheme.
//!
//! When a buyer cannot or does not want to pre-approve the canonical
//! Permit2 contract, they MAY attach a signed EIP-2612 permit to the
//! payment payload under the `eip2612GasSponsoring` extension key. The
//! facilitator then routes the settlement through
//! [`IX402UptoPermit2Proxy::settleWithPermit`](super::facilitator::contract::IX402UptoPermit2Proxy),
//! which atomically:
//!
//! 1. broadcasts the EIP-2612 `permit` against the token, granting the
//!    canonical Permit2 contract an allowance equal to
//!    `permit.permitted.amount`, and
//! 2. invokes the standard upto Permit2 settle path with the buyer's
//!    `PermitWitnessTransferFrom` signature.
//!
//! This module models only the wire-level [`Eip2612SignedPermit`]; the
//! verify and settle pipelines in [`super::facilitator`] consume it.

use alloy_primitives::{Address, Bytes};
use r402_core::wire::{ExtensionEntry, Extensions};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::chain::TokenAmount;

/// Stable extension identifier carried under [`PaymentPayload::extensions`].
///
/// Mirrors the constant used by the official TypeScript and Go reference
/// implementations so cross-stack interop works without translation.
///
/// [`PaymentPayload::extensions`]: r402_core::wire::Extensions
pub const EIP2612_GAS_SPONSORING_KEY: &str = "eip2612GasSponsoring";

/// Buyer-signed EIP-2612 permit that authorises a one-shot allowance to
/// the canonical Permit2 contract.
///
/// The signature is the 65-byte packed `(r, s, v)` form produced by an
/// EOA wallet over the standard EIP-2612 EIP-712 typed data:
///
/// ```text
/// EIP712Domain(string name, string version, uint256 chainId, address verifyingContract)
/// Permit(address owner, address spender, uint256 value, uint256 nonce, uint256 deadline)
/// ```
///
/// `name` and `version` come from the token contract (echoed via
/// [`UptoPaymentRequirementsExtra`](super::UptoPaymentRequirementsExtra)),
/// `verifyingContract` is `asset`, and `nonce` is fetched from
/// `IERC20Permit::nonces(owner)` at sign time.
///
/// The proxy enforces `value == permit.permitted.amount` and
/// `spender == canonical Permit2 address`, so the wire form mirrors the
/// official spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Eip2612SignedPermit {
    /// Permit owner (== payment payer / `payload.permit2_authorization.from`).
    pub from: Address,
    /// ERC-20 token granting the allowance (== `requirements.asset`).
    pub asset: Address,
    /// Permit spender — MUST be the canonical Permit2 address
    /// (`crate::exact::PERMIT2_ADDRESS`).
    pub spender: Address,
    /// Approved allowance — MUST equal
    /// `payload.permit2_authorization.permitted.amount` (the signed maximum).
    pub value: TokenAmount,
    /// Permit deadline (Unix seconds). Distinct from the Permit2 deadline.
    pub deadline: TokenAmount,
    /// 65-byte packed `(r, s, v)` EOA signature.
    pub signature: Bytes,
}

/// Errors raised while extracting [`Eip2612SignedPermit`] from a wire
/// extensions map.
#[derive(Debug, Error)]
pub enum Eip2612ParseError {
    /// The extension entry did not deserialise into the expected shape.
    #[error("invalid eip2612GasSponsoring extension payload: {0}")]
    Invalid(#[from] serde_json::Error),
    /// The signature was not 65 bytes (`r ‖ s ‖ v`).
    #[error("eip2612 signature MUST be 65 bytes long, got {0}")]
    InvalidSignatureLength(usize),
}

impl Eip2612SignedPermit {
    /// Length of the canonical packed `(r, s, v)` signature in bytes.
    pub const SIGNATURE_LEN: usize = 65;

    /// Looks up and decodes the EIP-2612 extension from a wire
    /// [`Extensions`] map.
    ///
    /// Returns:
    ///
    /// - `Ok(None)` when the extension is absent (the default Permit2-only
    ///   settle path applies);
    /// - `Ok(Some(_))` when the extension is present and well-formed;
    /// - `Err(_)` when the extension is present but malformed — callers
    ///   SHOULD reject the payload with
    ///   [`VerificationError::InvalidFormat`](r402_core::error::VerificationError::InvalidFormat).
    ///
    /// # Errors
    ///
    /// Returns [`Eip2612ParseError::Invalid`] when the extension JSON
    /// fails to deserialise into [`Self`]. Returns
    /// [`Eip2612ParseError::InvalidSignatureLength`] when the contained
    /// signature is not the canonical 65-byte `r ‖ s ‖ v` form.
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

    /// Splits the packed signature into `(r, s, v)` for ABI encoding.
    ///
    /// # Errors
    ///
    /// Never errors at runtime: [`Self::from_extensions`] enforces the
    /// 65-byte invariant on construction. Callers building permits by
    /// hand MUST ensure the signature is exactly 65 bytes.
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

    /// Builds an extension entry suitable for insertion into
    /// [`Extensions`].
    ///
    /// Uses the structured `{info}` envelope per the canonical x402
    /// extension layout.
    ///
    /// # Errors
    ///
    /// Returns a `serde_json::Error` only if `Self` cannot be serialised
    /// (impossible by construction; the type is composed of
    /// always-serialisable primitives).
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
            value: TokenAmount::from(U256::from(1_000_000_u64)),
            deadline: TokenAmount::from(U256::from(1_700_000_000_u64)),
            signature: Bytes::from(vec![0x42u8; 65]),
        }
    }

    #[test]
    fn extension_round_trips_through_structured_envelope() {
        let permit = sample_permit();
        let mut extensions = Extensions::new();
        extensions.insert(
            EIP2612_GAS_SPONSORING_KEY,
            permit.to_extension_entry().expect("serialises cleanly"),
        );
        let extracted = Eip2612SignedPermit::from_extensions(&extensions)
            .expect("structured payload decodes")
            .expect("extension present");
        assert_eq!(extracted, permit);
    }

    #[test]
    fn extension_round_trips_through_raw_envelope() {
        // Some clients may emit a raw JSON value rather than the
        // canonical {info} wrapper; the extractor MUST tolerate both.
        let permit = sample_permit();
        let mut extensions = Extensions::new();
        extensions.insert(
            EIP2612_GAS_SPONSORING_KEY,
            ExtensionEntry::raw(serde_json::to_value(&permit).unwrap()),
        );
        let extracted = Eip2612SignedPermit::from_extensions(&extensions)
            .expect("raw payload decodes")
            .expect("extension present");
        assert_eq!(extracted, permit);
    }

    #[test]
    fn missing_extension_returns_ok_none() {
        let extensions = Extensions::new();
        let extracted = Eip2612SignedPermit::from_extensions(&extensions).expect("no error");
        assert!(extracted.is_none());
    }

    #[test]
    fn malformed_extension_returns_invalid_error() {
        let mut extensions = Extensions::new();
        extensions.insert(
            EIP2612_GAS_SPONSORING_KEY,
            ExtensionEntry::raw(json!({ "from": "not-an-address" })),
        );
        let err = Eip2612SignedPermit::from_extensions(&extensions).unwrap_err();
        assert!(matches!(err, Eip2612ParseError::Invalid(_)));
    }

    #[test]
    fn signature_length_validation_rejects_short_sigs() {
        let mut permit = sample_permit();
        permit.signature = Bytes::from(vec![0u8; 64]);
        let mut extensions = Extensions::new();
        extensions.insert(
            EIP2612_GAS_SPONSORING_KEY,
            permit.to_extension_entry().unwrap(),
        );
        let err = Eip2612SignedPermit::from_extensions(&extensions).unwrap_err();
        assert!(matches!(err, Eip2612ParseError::InvalidSignatureLength(64)));
    }

    #[test]
    fn split_signature_returns_components_in_order() {
        let mut sig_bytes = Vec::with_capacity(65);
        sig_bytes.extend_from_slice(&[0x11u8; 32]);
        sig_bytes.extend_from_slice(&[0x22u8; 32]);
        sig_bytes.push(27);
        let permit = Eip2612SignedPermit {
            signature: Bytes::from(sig_bytes),
            ..sample_permit()
        };
        let (r, s, v) = permit.split_signature().expect("65-byte signature");
        assert_eq!(r.as_slice(), [0x11u8; 32]);
        assert_eq!(s.as_slice(), [0x22u8; 32]);
        assert_eq!(v, 27);
    }
}
