//! `SignatureChecker`: getCode then ECDSA (empty) or EIP-1271 (code). No ECDSA fallback.

use alloy_primitives::{Address, B256, Bytes, FixedBytes, hex};
use alloy_provider::Provider;
use r402_protocol::error::VerificationError;

use crate::chain::contracts::IERC1271;
use crate::signature::{ClassifiedSignature, StructuredSignature, classify_with_code};

const ERC1271_MAGIC: FixedBytes<4> = FixedBytes(hex!("1626ba7e"));

pub(super) async fn verify_hash_signature<P: Provider>(
    provider: &P,
    signer: Address,
    digest: B256,
    signature: Bytes,
    invalid_reason: &'static str,
) -> Result<(), VerificationError> {
    let structured = StructuredSignature::try_from_bytes(signature)
        .map_err(|_| VerificationError::from_wire(invalid_reason))?;
    let classified = classify_with_code(provider, signer, structured, &digest)
        .await
        .map_err(|_| VerificationError::from_wire(invalid_reason))?;
    match classified {
        ClassifiedSignature::Eoa(_) => Ok(()),
        ClassifiedSignature::EIP1271(sig) => {
            let contract = IERC1271::new(signer, provider);
            match contract.isValidSignature(digest, sig).call().await {
                Ok(magic) if magic == ERC1271_MAGIC => Ok(()),
                _ => Err(VerificationError::from_wire(invalid_reason)),
            }
        }
        ClassifiedSignature::EIP6492 { inner, .. } => {
            Box::pin(verify_hash_signature(
                provider,
                signer,
                digest,
                inner,
                invalid_reason,
            ))
            .await
        }
    }
}
