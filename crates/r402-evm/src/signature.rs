//! Shared EOA / EIP-1271 / EIP-6492 signature parsing and payment time windows.

use alloy_primitives::{Address, B256, Bytes, Signature, hex};
use alloy_sol_types::SolType;
use r402_core::error::VerificationError;
use r402_core::wire::UnixTimestamp;
#[cfg(feature = "telemetry")]
use tracing::instrument;

use crate::chain::contracts::Sig6492;

/// The fixed 32-byte magic suffix defined by [EIP-6492](https://eips.ethereum.org/EIPS/eip-6492).
const EIP6492_MAGIC_SUFFIX: [u8; 32] =
    hex!("6492649264926492649264926492649264926492649264926492649264926492");

/// A structured representation of an Ethereum signature.
///
/// This enum normalizes two supported cases:
/// - **EIP-6492 wrapped signatures**: used for counterfactual contract wallets.
/// - **EIP-1271 signatures**: plain contract (or EOA-style) signatures.
#[derive(Debug, Clone)]
pub(crate) enum StructuredSignature {
    /// An EIP-6492 wrapped signature.
    EIP6492 {
        factory: Address,
        factory_calldata: Bytes,
        inner: Bytes,
        original: Bytes,
    },
    /// Normalized EOA signature.
    Eoa(Signature),
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

/// Decodes the EIP-6492 wrapper from raw signature bytes.
///
/// Returns `Some(EIP6492 { .. })` if the bytes end with the 32-byte magic
/// suffix, or `None` if no wrapper is present.
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
    Ok(Some(StructuredSignature::EIP6492 {
        factory: sig6492.factory,
        factory_calldata: sig6492.factoryCalldata,
        inner: sig6492.innerSig,
        original: bytes,
    }))
}

impl StructuredSignature {
    pub(crate) fn try_from_bytes(
        bytes: Bytes,
        expected_signer: Address,
        prehash: &B256,
    ) -> Result<Self, StructuredSignatureFormatError> {
        if let Some(eip6492) = decode_eip6492(bytes.clone())? {
            return Ok(eip6492);
        }
        let eoa_signature = if bytes.len() == 65 {
            Signature::from_raw(&bytes)
                .ok()
                .map(Signature::normalized_s)
        } else if bytes.len() == 64 {
            Some(Signature::from_erc2098(&bytes).normalized_s())
        } else {
            None
        };
        let signature = match eoa_signature {
            None => Self::EIP1271(bytes),
            Some(s) => {
                let is_expected_signer = s
                    .recover_address_from_prehash(prehash)
                    .is_ok_and(|r| r == expected_signer);
                if is_expected_signer {
                    Self::Eoa(s)
                } else {
                    Self::EIP1271(bytes)
                }
            }
        };
        Ok(signature)
    }
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
