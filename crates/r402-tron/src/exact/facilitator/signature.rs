//! Signature parsing and verification for Tron exact scheme.
//!
//! Tron has no contract wallets, so only EOA secp256k1 signatures are
//! supported — unlike `r402-evm`, EIP-1271 / EIP-6492 do not apply.

use alloy_primitives::{Address, B256, Signature, U256};
use alloy_sol_types::{Eip712Domain, SolStruct};

use super::Eip3009Payment;
use crate::exact::TransferWithAuthorization;

/// Canonical data required to verify a Tron EOA signature.
#[derive(Debug, Clone)]
pub(crate) struct SignedMessage {
    /// Expected signer (EOA).
    pub address: Address,
}

impl SignedMessage {
    /// Constructs a [`SignedMessage`] from an [`Eip3009Payment`] and its
    /// corresponding [`Eip712Domain`].
    pub(super) fn extract(
        payment: &Eip3009Payment,
        domain: &Eip712Domain,
    ) -> Result<Self, TronSignatureError> {
        let transfer_with_authorization = TransferWithAuthorization {
            from: payment.from,
            to: payment.to,
            value: payment.value,
            validAfter: U256::from(payment.valid_after.as_secs()),
            validBefore: U256::from(payment.valid_before.as_secs()),
            nonce: payment.nonce,
        };
        let eip712_hash = transfer_with_authorization.eip712_signing_hash(domain);
        let _ = parse_eoa_signature(&payment.signature, &eip712_hash, payment.from)?;
        Ok(Self {
            address: payment.from,
        })
    }
}

/// Errors from parsing a Tron EOA signature.
#[derive(Debug, thiserror::Error)]
pub enum TronSignatureError {
    /// Signature bytes are not 65 or 64 bytes long.
    #[error("invalid signature length: {0} bytes (expected 65)")]
    InvalidLength(usize),
    /// Signature could not be parsed as a valid secp256k1 signature.
    #[error("invalid signature encoding: {0}")]
    InvalidEncoding(String),
    /// Recovered address does not match the declared sender.
    #[error("signature does not recover to the declared sender")]
    SignerMismatch,
}

/// Parses raw signature bytes into a normalized EOA [`Signature`], verifying
/// that it recovers to `expected_signer`.
///
/// Tron only supports plain EOA signatures (65 bytes: r(32) + s(32) + v(1)),
/// or 64-byte compact (ERC-2098) form. No EIP-1271 / EIP-6492 wrapping.
fn parse_eoa_signature(
    bytes: &[u8],
    prehash: &B256,
    expected_signer: Address,
) -> Result<Signature, TronSignatureError> {
    let signature = if bytes.len() == 65 {
        Signature::from_raw(bytes)
            .map_err(|e| TronSignatureError::InvalidEncoding(e.to_string()))?
            .normalized_s()
    } else if bytes.len() == 64 {
        Signature::from_erc2098(bytes).normalized_s()
    } else {
        return Err(TronSignatureError::InvalidLength(bytes.len()));
    };
    let recovered = signature
        .recover_address_from_prehash(prehash)
        .map_err(|e| TronSignatureError::InvalidEncoding(e.to_string()))?;
    if recovered != expected_signer {
        return Err(TronSignatureError::SignerMismatch);
    }
    Ok(signature)
}
