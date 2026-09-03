//! Cumulative voucher signing and EOA recovery.

#[cfg(feature = "client")]
use alloy_primitives::Bytes;
use alloy_primitives::{Address, B256, Signature, U256};
use alloy_sol_types::{SolStruct, eip712_domain, sol};
#[cfg(feature = "client")]
use r402_protocol::error::ClientError;
use r402_protocol::error::VerificationError;

use super::payload::{
    BATCH_SETTLEMENT_ADDRESS, BATCH_SETTLEMENT_DOMAIN_NAME, BATCH_SETTLEMENT_DOMAIN_VERSION,
    VoucherFields,
};
#[cfg(feature = "client")]
use crate::chain::TokenAmount;
#[cfg(feature = "client")]
use crate::signer::SignerLike;

sol! {
    /// `Voucher(bytes32 channelId, uint128 maxClaimableAmount)`.
    struct Voucher {
        bytes32 channelId;
        uint128 maxClaimableAmount;
    }
}

fn domain(chain_id: u64) -> alloy_sol_types::Eip712Domain {
    eip712_domain! {
        name: BATCH_SETTLEMENT_DOMAIN_NAME,
        version: BATCH_SETTLEMENT_DOMAIN_VERSION,
        chain_id: chain_id,
        verifying_contract: BATCH_SETTLEMENT_ADDRESS,
    }
}

/// Signs a cumulative voucher under the batch-settlement domain.
///
/// # Errors
///
/// Wallet signing failure.
#[cfg(feature = "client")]
pub async fn sign_voucher<S: SignerLike + Sync>(
    signer: &S,
    channel_id: B256,
    max_claimable_amount: U256,
    chain_id: u64,
) -> Result<VoucherFields, ClientError> {
    let max_u128 = u128::try_from(max_claimable_amount)
        .map_err(|_| ClientError::Signing("maxClaimableAmount exceeds uint128".into()))?;
    let typed = Voucher {
        channelId: channel_id,
        maxClaimableAmount: max_u128,
    };
    let hash = typed.eip712_signing_hash(&domain(chain_id));
    let signature = signer
        .sign_hash(&hash)
        .await
        .map_err(|e| ClientError::Signing(format!("{e:?}")))?;
    Ok(VoucherFields {
        channel_id,
        max_claimable_amount: TokenAmount::from(max_claimable_amount),
        signature: Bytes::from(signature.as_bytes().to_vec()),
    })
}

/// Recovers the voucher signer (EOA 65-byte path) and checks expected authorizer.
///
/// When `payer_authorizer` is zero, recovers against `payer` (EIP-1271 deferred
/// to on-chain for non-65-byte signatures).
///
/// # Errors
///
/// Invalid signature or unexpected signer.
pub fn verify_voucher_signature(
    voucher: &VoucherFields,
    chain_id: u64,
    payer: Address,
    payer_authorizer: Address,
) -> Result<Address, VerificationError> {
    let max_u128 = u128::try_from(voucher.max_claimable_amount.0).map_err(|_| {
        VerificationError::InvalidFormat("maxClaimableAmount exceeds uint128".into())
    })?;
    let typed = Voucher {
        channelId: voucher.channel_id,
        maxClaimableAmount: max_u128,
    };
    let hash = typed.eip712_signing_hash(&domain(chain_id));
    let expected = if payer_authorizer == Address::ZERO {
        payer
    } else {
        payer_authorizer
    };

    if voucher.signature.len() != 65 {
        // Request-path settle never goes on-chain; accepting non-EOA blobs
        // would allow free access. Smart-wallet EIP-1271 belongs on claim.
        return Err(VerificationError::InvalidSignature(
            "batch-settlement request path requires 65-byte EOA voucher signature".into(),
        ));
    }

    let sig = Signature::from_raw(voucher.signature.as_ref())
        .map_err(|e| VerificationError::InvalidSignature(format!("parse: {e}")))?;
    let recovered = sig
        .recover_address_from_prehash(&hash)
        .map_err(|e| VerificationError::InvalidSignature(format!("recover: {e}")))?;
    if recovered != expected {
        return Err(VerificationError::InvalidSignature(
            "voucher signer mismatch".into(),
        ));
    }
    Ok(recovered)
}

#[cfg(test)]
#[cfg(feature = "client")]
mod tests {
    use alloy_primitives::Address;
    use alloy_signer_local::PrivateKeySigner;

    use super::*;
    use crate::batch_settlement::channel::compute_channel_id;
    use crate::batch_settlement::payload::ChannelConfig;

    #[tokio::test]
    async fn sign_and_verify_voucher() {
        let signer = PrivateKeySigner::random();
        let payer = signer.address();
        let cfg = ChannelConfig {
            payer,
            payer_authorizer: payer,
            receiver: Address::repeat_byte(0x22),
            receiver_authorizer: Address::repeat_byte(0x33),
            token: Address::repeat_byte(0x44),
            withdraw_delay: 900,
            salt: B256::ZERO,
        };
        let channel_id = compute_channel_id(&cfg, 8453).expect("id");
        let voucher = sign_voucher(&signer, channel_id, U256::from(1000_u64), 8453)
            .await
            .expect("sign");
        let recovered = verify_voucher_signature(&voucher, 8453, payer, payer).expect("verify");
        assert_eq!(recovered, payer);
    }
}
