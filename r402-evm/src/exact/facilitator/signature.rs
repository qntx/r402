//! Signature parsing and verification types.
//!
//! Handles EOA, EIP-1271 (contract wallet), and EIP-6492 (counterfactual wallet)
//! signature formats used in ERC-3009 payment authorization.

use alloy_primitives::{Address, B256, Bytes, Signature, hex};
use alloy_sol_types::{SolStruct, SolType};

use super::Eip3009Payment;
use super::contract::Sig6492;
use crate::exact::TransferWithAuthorization;

/// The fixed 32-byte magic suffix defined by [EIP-6492](https://eips.ethereum.org/EIPS/eip-6492).
const EIP6492_MAGIC_SUFFIX: [u8; 32] =
    hex!("6492649264926492649264926492649264926492649264926492649264926492");

/// Canonical data required to verify a signature.
#[derive(Debug, Clone)]
pub(super) struct SignedMessage {
    /// Expected signer (an EOA or contract wallet).
    pub address: Address,
    /// 32-byte digest that was signed (typically an EIP-712 hash).
    pub hash: B256,
    /// Structured signature, either EIP-6492 or EIP-1271.
    pub signature: StructuredSignature,
}

impl SignedMessage {
    /// Construct a [`SignedMessage`] from an [`Eip3009Payment`] and its
    /// corresponding [`Eip712Domain`](alloy_sol_types::Eip712Domain).
    pub fn extract(
        payment: &Eip3009Payment,
        domain: &alloy_sol_types::Eip712Domain,
    ) -> Result<Self, StructuredSignatureFormatError> {
        let transfer_with_authorization = TransferWithAuthorization {
            from: payment.from,
            to: payment.to,
            value: payment.value,
            validAfter: alloy_primitives::U256::from(payment.valid_after.as_secs()),
            validBefore: alloy_primitives::U256::from(payment.valid_before.as_secs()),
            nonce: payment.nonce,
        };
        let eip712_hash = transfer_with_authorization.eip712_signing_hash(domain);
        let structured_signature: StructuredSignature = StructuredSignature::try_from_bytes(
            payment.signature.clone(),
            payment.from,
            &eip712_hash,
        )?;
        let signed_message = Self {
            address: payment.from,
            hash: eip712_hash,
            signature: structured_signature,
        };
        Ok(signed_message)
    }
}

/// A structured representation of an Ethereum signature.
///
/// This enum normalizes two supported cases:
/// - **EIP-6492 wrapped signatures**: used for counterfactual contract wallets.
/// - **EIP-1271 signatures**: plain contract (or EOA-style) signatures.
#[derive(Debug, Clone)]
pub(super) enum StructuredSignature {
    /// An EIP-6492 wrapped signature.
    EIP6492 {
        factory: Address,
        factory_calldata: Bytes,
        inner: Bytes,
        original: Bytes,
    },
    /// Normalized EOA signature.
    #[allow(clippy::upper_case_acronyms)]
    EOA(Signature),
    /// A plain EIP-1271 or EOA signature (no 6492 wrappers).
    EIP1271(Bytes),
}

/// Errors from parsing a structured signature.
#[derive(Debug, thiserror::Error)]
pub enum StructuredSignatureFormatError {
    /// The EIP-6492 wrapper could not be decoded.
    #[error(transparent)]
    InvalidEIP6492Format(alloy_sol_types::Error),
}

impl StructuredSignature {
    pub fn try_from_bytes(
        bytes: Bytes,
        expected_signer: Address,
        prehash: &B256,
    ) -> Result<Self, StructuredSignatureFormatError> {
        let is_eip6492 = bytes.len() >= 32 && bytes[bytes.len() - 32..] == EIP6492_MAGIC_SUFFIX;
        let signature = if is_eip6492 {
            let body = &bytes[..bytes.len() - 32];
            let sig6492 = Sig6492::abi_decode_params(body)
                .map_err(StructuredSignatureFormatError::InvalidEIP6492Format)?;
            Self::EIP6492 {
                factory: sig6492.factory,
                factory_calldata: sig6492.factoryCalldata,
                inner: sig6492.innerSig,
                original: bytes,
            }
        } else {
            let eoa_signature = if bytes.len() == 65 {
                Signature::from_raw(&bytes)
                    .ok()
                    .map(Signature::normalized_s)
            } else if bytes.len() == 64 {
                Some(Signature::from_erc2098(&bytes).normalized_s())
            } else {
                None
            };
            match eoa_signature {
                None => Self::EIP1271(bytes),
                Some(s) => {
                    let is_expected_signer = s
                        .recover_address_from_prehash(prehash)
                        .ok()
                        .is_some_and(|r| r == expected_signer);
                    if is_expected_signer {
                        Self::EOA(s)
                    } else {
                        Self::EIP1271(bytes)
                    }
                }
            }
        };
        Ok(signature)
    }
}

impl TryFrom<Bytes> for StructuredSignature {
    type Error = StructuredSignatureFormatError;

    fn try_from(bytes: Bytes) -> Result<Self, Self::Error> {
        let is_eip6492 = bytes.len() >= 32 && bytes[bytes.len() - 32..] == EIP6492_MAGIC_SUFFIX;
        let signature = if is_eip6492 {
            let body = &bytes[..bytes.len() - 32];
            let sig6492 = Sig6492::abi_decode_params(body)
                .map_err(StructuredSignatureFormatError::InvalidEIP6492Format)?;
            Self::EIP6492 {
                factory: sig6492.factory,
                factory_calldata: sig6492.factoryCalldata,
                inner: sig6492.innerSig,
                original: bytes,
            }
        } else {
            Self::EIP1271(bytes)
        };
        Ok(signature)
    }
}
