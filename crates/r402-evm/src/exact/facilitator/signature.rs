//! Exact-scheme EIP-3009 signed-message extraction.
//!
//! Generic EOA / EIP-1271 / EIP-6492 parsing lives in [`crate::signature`].

use alloy_primitives::{Address, B256};
use alloy_sol_types::SolStruct;

use super::Eip3009Payment;
use crate::exact::TransferWithAuthorization;
use crate::signature::{StructuredSignature, StructuredSignatureFormatError};

/// Canonical data required to verify an ERC-3009 signature.
#[derive(Debug, Clone)]
pub(crate) struct SignedMessage {
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
    pub(super) fn extract(
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
        Ok(Self {
            address: payment.from,
            hash: eip712_hash,
            signature: structured_signature,
        })
    }
}
