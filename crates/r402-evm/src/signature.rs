//! Shared EOA / EIP-1271 / EIP-6492 signature parsing and payment time windows.

use alloy_primitives::{Address, B256, Bytes, Signature, hex};
use alloy_provider::Provider;
use alloy_sol_types::SolType;
use r402_protocol::error::VerificationError;
use r402_protocol::payment::UnixTimestamp;
#[cfg(feature = "telemetry")]
use tracing::instrument;

use crate::chain::contracts::Sig6492;
use crate::error::Eip155ExactError;

/// The fixed 32-byte magic suffix defined by [EIP-6492](https://eips.ethereum.org/EIPS/eip-6492).
pub(crate) const EIP6492_MAGIC_SUFFIX: [u8; 32] =
    hex!("6492649264926492649264926492649264926492649264926492649264926492");

/// Parsed signature wrapper. Not yet classified as EOA vs EIP-1271.
#[derive(Debug, Clone)]
#[allow(
    variant_size_differences,
    reason = "6492 wrapper carries factory calldata plus inner/original bytes"
)]
pub(crate) enum StructuredSignature {
    /// An EIP-6492 wrapped signature with a non-zero factory.
    EIP6492 {
        factory: Address,
        factory_calldata: Bytes,
        inner: Bytes,
        original: Bytes,
    },
    /// Raw bytes; verify/settle must `getCode(payer)` then branch.
    Unclassified(Bytes),
}

/// Signature kind used by `transferWithAuthorization` / Permit2 call builders.
#[derive(Debug, Clone)]
#[allow(
    variant_size_differences,
    reason = "6492 wrapper carries factory calldata plus inner/original bytes"
)]
pub(crate) enum ClassifiedSignature {
    /// EIP-6492 wrapper. Counterfactual vs deployed is a separate `getCode(payer)`.
    EIP6492 {
        factory: Address,
        factory_calldata: Bytes,
        inner: Bytes,
        original: Bytes,
    },
    /// Normalized EOA signature (no code at payer; recover matches payer).
    Eoa(Signature),
    /// Contract (or 7702-delegated) signature bytes. Never recovered as ECDSA.
    EIP1271(Bytes),
}

/// Errors from parsing a structured signature.
#[derive(Debug, thiserror::Error)]
pub enum StructuredSignatureFormatError {
    /// The EIP-6492 wrapper could not be decoded.
    #[error(transparent)]
    InvalidEIP6492Format(alloy_sol_types::Error),
}

/// Decodes the EIP-6492 wrapper from raw signature bytes.
///
/// Returns `Some` if the bytes end with the 32-byte magic suffix, or `None`
/// if no wrapper is present.
#[allow(
    clippy::indexing_slicing,
    reason = "bounds checked by len() >= 32 guard"
)]
fn decode_eip6492(
    bytes: Bytes,
) -> Result<Option<StructuredSignature>, StructuredSignatureFormatError> {
    let has_suffix = bytes.len() >= 32 && bytes[bytes.len() - 32..] == EIP6492_MAGIC_SUFFIX;
    if !has_suffix {
        return Ok(None);
    }
    let body = &bytes[..bytes.len() - 32];
    let sig6492 = Sig6492::abi_decode_params(body)
        .map_err(StructuredSignatureFormatError::InvalidEIP6492Format)?;
    if sig6492.factory.is_zero() {
        return Ok(None);
    }
    Ok(Some(StructuredSignature::EIP6492 {
        factory: sig6492.factory,
        factory_calldata: sig6492.factoryCalldata,
        inner: sig6492.innerSig,
        original: bytes,
    }))
}

impl StructuredSignature {
    /// Unwraps an EIP-6492 magic suffix. Does not recover ECDSA or classify EOA vs 1271.
    pub(crate) fn try_from_bytes(bytes: Bytes) -> Result<Self, StructuredSignatureFormatError> {
        if let Some(eip6492) = decode_eip6492(bytes.clone())? {
            return Ok(eip6492);
        }
        Ok(Self::Unclassified(bytes))
    }
}

/// Classifies `sig` from already-fetched bytecode. `code == None` is a getCode failure.
pub(crate) fn classify_from_code(
    payer: Address,
    sig: StructuredSignature,
    prehash: &B256,
    code: Option<&[u8]>,
) -> Result<ClassifiedSignature, Eip155ExactError> {
    match sig {
        StructuredSignature::EIP6492 {
            factory,
            factory_calldata,
            inner,
            original,
        } => Ok(ClassifiedSignature::EIP6492 {
            factory,
            factory_calldata,
            inner,
            original,
        }),
        StructuredSignature::Unclassified(bytes) => {
            let Some(code) = code else {
                return Err(
                    VerificationError::InvalidSignature("eth_getCode failed".into()).into(),
                );
            };
            if code.is_empty() {
                classify_eoa(payer, &bytes, prehash)
            } else {
                Ok(ClassifiedSignature::EIP1271(bytes))
            }
        }
    }
}

fn classify_eoa(
    payer: Address,
    bytes: &Bytes,
    prehash: &B256,
) -> Result<ClassifiedSignature, Eip155ExactError> {
    let eoa_signature = if bytes.len() == 65 {
        Signature::from_raw(bytes).ok().map(Signature::normalized_s)
    } else if bytes.len() == 64 {
        Some(Signature::from_erc2098(bytes).normalized_s())
    } else {
        None
    };
    let Some(signature) = eoa_signature else {
        return Err(VerificationError::InvalidSignature(
            "empty-code payer requires a 64/65-byte ECDSA signature".into(),
        )
        .into());
    };
    let recovered = signature
        .recover_address_from_prehash(prehash)
        .map_err(|e| VerificationError::InvalidSignature(e.to_string()))?;
    if recovered != payer {
        return Err(VerificationError::InvalidSignature(
            "recovered signer does not match payer".into(),
        )
        .into());
    }
    Ok(ClassifiedSignature::Eoa(signature))
}

/// `eth_getCode` then branch. No ECDSA fallback when code is present.
pub(crate) async fn classify_with_code<P: Provider>(
    provider: &P,
    payer: Address,
    sig: StructuredSignature,
    prehash: &B256,
) -> Result<ClassifiedSignature, Eip155ExactError> {
    if matches!(sig, StructuredSignature::EIP6492 { .. }) {
        return classify_from_code(payer, sig, prehash, Some(&[]));
    }
    let Ok(code) = provider.get_code_at(payer).await else {
        return classify_from_code(payer, sig, prehash, None);
    };
    classify_from_code(payer, sig, prehash, Some(code.as_ref()))
}

/// Checks that `now` sits inside `[valid_after, valid_before)` with skew.
///
/// Applies `clock_skew_tolerance` seconds of grace when checking both expiration
/// and early-arrival to account for clock drift between nodes.
///
/// # Errors
///
/// Returns [`VerificationError::Expired`] or [`VerificationError::Early`].
#[cfg_attr(feature = "telemetry", instrument(skip_all, err))]
pub(crate) fn assert_time(
    valid_after: UnixTimestamp,
    valid_before: UnixTimestamp,
    clock_skew_tolerance: u64,
) -> Result<(), VerificationError> {
    let now = UnixTimestamp::now();
    if valid_before < now + clock_skew_tolerance {
        return Err(VerificationError::Expired);
    }
    if valid_after > now + clock_skew_tolerance {
        return Err(VerificationError::Early);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use alloy_signer::SignerSync;
    use alloy_signer_local::PrivateKeySigner;
    use alloy_sol_types::SolValue;

    use super::*;

    fn signed_eoa() -> (Address, B256, Bytes, Signature) {
        let signer = PrivateKeySigner::from_slice(&[0x42u8; 32]).expect("key");
        let payer = signer.address();
        let prehash = B256::repeat_byte(0x11);
        let signature = signer.sign_hash_sync(&prehash).expect("sign");
        (
            payer,
            prehash,
            Bytes::from(signature.as_bytes().to_vec()),
            signature,
        )
    }

    fn wrap_6492(factory: Address, inner: Bytes) -> Bytes {
        let encoded = Sig6492 {
            factory,
            factoryCalldata: Bytes::from(vec![0xde, 0xad]),
            innerSig: inner,
        }
        .abi_encode_params();
        let mut out = encoded;
        out.extend_from_slice(&EIP6492_MAGIC_SUFFIX);
        Bytes::from(out)
    }

    #[test]
    fn try_from_bytes_does_not_classify_eoa() {
        let (_payer, _prehash, bytes, _) = signed_eoa();
        let parsed = StructuredSignature::try_from_bytes(bytes.clone()).unwrap();
        assert!(matches!(parsed, StructuredSignature::Unclassified(b) if b == bytes));
    }

    #[test]
    fn try_from_bytes_unwraps_6492_without_recover() {
        let factory = Address::repeat_byte(0xF1);
        let inner = Bytes::from(vec![0u8; 65]);
        let wrapped = wrap_6492(factory, inner.clone());
        let parsed = StructuredSignature::try_from_bytes(wrapped.clone()).unwrap();
        match parsed {
            StructuredSignature::EIP6492 {
                factory: f,
                inner: i,
                original,
                ..
            } => {
                assert_eq!(f, factory);
                assert_eq!(i, inner);
                assert_eq!(original, wrapped);
            }
            StructuredSignature::Unclassified(_) => panic!("expected EIP6492"),
        }
    }

    #[test]
    fn zero_factory_6492_is_unclassified() {
        let inner = Bytes::from(vec![0u8; 65]);
        let wrapped = wrap_6492(Address::ZERO, inner);
        let parsed = StructuredSignature::try_from_bytes(wrapped.clone()).unwrap();
        assert!(matches!(parsed, StructuredSignature::Unclassified(b) if b == wrapped));
    }

    #[test]
    fn empty_code_matching_ecdsa_is_eoa() {
        let (payer, prehash, bytes, _) = signed_eoa();
        let parsed = StructuredSignature::try_from_bytes(bytes).unwrap();
        let classified = classify_from_code(payer, parsed, &prehash, Some(&[])).unwrap();
        assert!(matches!(classified, ClassifiedSignature::Eoa(_)));
    }

    #[test]
    fn code_present_owner_ecdsa_is_eip1271() {
        let (payer, prehash, bytes, _) = signed_eoa();
        let parsed = StructuredSignature::try_from_bytes(bytes.clone()).unwrap();
        let classified = classify_from_code(payer, parsed, &prehash, Some(&[0xef, 0x01])).unwrap();
        match classified {
            ClassifiedSignature::EIP1271(raw) => assert_eq!(raw, bytes),
            other => panic!("expected EIP1271, got {other:?}"),
        }
    }

    #[test]
    fn getcode_error_is_invalid() {
        let (payer, prehash, bytes, _) = signed_eoa();
        let parsed = StructuredSignature::try_from_bytes(bytes).unwrap();
        let err = classify_from_code(payer, parsed, &prehash, None).unwrap_err();
        assert!(matches!(
            err,
            Eip155ExactError::PaymentVerification(VerificationError::InvalidSignature(_))
        ));
    }

    #[test]
    fn empty_code_recover_mismatch_is_invalid() {
        let (payer, prehash, bytes, _) = signed_eoa();
        let parsed = StructuredSignature::try_from_bytes(bytes).unwrap();
        let other = Address::repeat_byte(0x99);
        assert_ne!(other, payer);
        let err = classify_from_code(other, parsed, &prehash, Some(&[])).unwrap_err();
        assert!(matches!(
            err,
            Eip155ExactError::PaymentVerification(VerificationError::InvalidSignature(_))
        ));
    }

    #[test]
    fn classify_6492_ignores_code() {
        let factory = Address::repeat_byte(0xF1);
        let wrapped = wrap_6492(factory, Bytes::from(vec![1u8; 65]));
        let parsed = StructuredSignature::try_from_bytes(wrapped).unwrap();
        let classified = classify_from_code(
            Address::repeat_byte(0xAA),
            parsed,
            &B256::ZERO,
            Some(&[0xef]),
        )
        .unwrap();
        assert!(matches!(
            classified,
            ClassifiedSignature::EIP6492 { factory: f, .. } if f == factory
        ));
    }

    #[test]
    fn eoa_rejects_non_ecdsa_length() {
        let bytes = Bytes::from(vec![0u8; 66]);
        let parsed = StructuredSignature::try_from_bytes(bytes).unwrap();
        let err = classify_from_code(Address::repeat_byte(0x01), parsed, &B256::ZERO, Some(&[]))
            .unwrap_err();
        assert!(matches!(
            err,
            Eip155ExactError::PaymentVerification(VerificationError::InvalidSignature(_))
        ));
    }
}
