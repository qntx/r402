//! Collector-data ABI encoding and Solidity tuple conversion.

use alloy_primitives::{B256, Bytes, U256, Uint, keccak256};
use alloy_sol_types::SolValue;
use r402_protocol::error::{FacilitatorError, VerificationError};

use super::contract::IBatchSettlement;
use crate::batch_settlement::payload::{ChannelConfig, VoucherClaim};

pub(super) fn erc3009_deposit_nonce(channel_id: B256, salt: B256) -> B256 {
    keccak256((channel_id, U256::from_be_bytes(salt.0)).abi_encode())
}

pub(super) fn erc3009_collector_data(
    valid_after: U256,
    valid_before: U256,
    salt: B256,
    signature: Bytes,
) -> Bytes {
    Bytes::from(
        (
            valid_after,
            valid_before,
            U256::from_be_bytes(salt.0),
            signature,
        )
            .abi_encode(),
    )
}

pub(super) fn permit2_collector_data(
    nonce: U256,
    deadline: U256,
    signature: Bytes,
    eip2612: Bytes,
) -> Bytes {
    Bytes::from((nonce, deadline, signature, eip2612).abi_encode())
}

pub(super) fn u128_amount(value: U256) -> Result<u128, FacilitatorError> {
    u128::try_from(value).map_err(|_| {
        FacilitatorError::Verification(VerificationError::InvalidFormat(
            "amount exceeds uint128".into(),
        ))
    })
}

pub(super) fn to_sol_config(
    cfg: &ChannelConfig,
) -> Result<IBatchSettlement::ChannelConfig, FacilitatorError> {
    let withdraw_delay = Uint::<40, 1>::try_from(cfg.withdraw_delay).map_err(|_| {
        FacilitatorError::Verification(VerificationError::InvalidFormat(
            "withdrawDelay exceeds uint40".into(),
        ))
    })?;
    Ok(IBatchSettlement::ChannelConfig {
        payer: cfg.payer,
        payerAuthorizer: cfg.payer_authorizer,
        receiver: cfg.receiver,
        receiverAuthorizer: cfg.receiver_authorizer,
        token: cfg.token,
        withdrawDelay: withdraw_delay,
        salt: cfg.salt,
    })
}

pub(super) fn to_sol_claims(
    claims: &[VoucherClaim],
) -> Result<Vec<IBatchSettlement::VoucherClaim>, FacilitatorError> {
    claims
        .iter()
        .map(|claim| {
            Ok(IBatchSettlement::VoucherClaim {
                voucher: IBatchSettlement::Voucher {
                    channel: to_sol_config(&claim.voucher.channel)?,
                    maxClaimableAmount: u128_amount(claim.voucher.max_claimable_amount.0)?,
                },
                signature: claim.signature.clone(),
                totalClaimed: u128_amount(claim.total_claimed.0)?,
            })
        })
        .collect()
}
