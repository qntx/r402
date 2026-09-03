//! On-chain `refundWithSignature`, optionally batched with claim via `multicall`.

use alloy_primitives::Bytes;
use alloy_provider::Provider;
use alloy_sol_types::SolCall;
use compact_str::CompactString;
use r402_protocol::error::{FacilitatorError, VerificationError};
use r402_protocol::payment as wire;

use super::Eip155BatchSettlementFacilitator;
use super::contract::IBatchSettlement;
use super::encoding::{to_sol_claims, to_sol_config, u128_amount};
use super::response::{facilitator_err_to_settle, settle_failure};
use super::send::{Broadcast, simulate_and_broadcast};
use super::validate::validate_channel_config;
use crate::batch_settlement::errors::{
    AUTHORIZER_NOT_CONFIGURED, CHANNEL_NOT_FOUND, CUMULATIVE_BELOW_CLAIMED,
    CUMULATIVE_EXCEEDS_BALANCE, INVALID_PAYLOAD_TYPE, INVALID_VOUCHER_SIGNATURE, REFUND_NO_BALANCE,
    REFUND_SIMULATION_FAILED, REFUND_TRANSACTION_FAILED, RPC_READ_FAILED,
};
use crate::batch_settlement::payload::{
    BATCH_SETTLEMENT_ADDRESS, BatchSettlementPayload, ChannelConfig, v2,
};
use crate::batch_settlement::voucher::verify_voucher_signature;
use crate::chain::Eip155MetaTransactionProvider;
use crate::error::Eip155ExactError;

pub(super) async fn verify_refund<P>(
    fac: &Eip155BatchSettlementFacilitator<P>,
    payload: &v2::PaymentPayload,
    requirements: &v2::PaymentRequirements,
    chain_id: u64,
) -> Result<wire::VerifyResponse, FacilitatorError>
where
    P: Eip155MetaTransactionProvider + Sync,
    P::Inner: Provider,
{
    let config = payload
        .payload
        .channel_config()
        .ok_or_else(|| VerificationError::from_wire(INVALID_PAYLOAD_TYPE))?;
    let voucher = payload
        .payload
        .voucher()
        .ok_or_else(|| VerificationError::from_wire(INVALID_PAYLOAD_TYPE))?;
    validate_channel_config(config, voucher.channel_id, requirements, chain_id)?;
    verify_voucher_signature(voucher, chain_id, config.payer, config.payer_authorizer)
        .map_err(|_| VerificationError::from_wire(INVALID_VOUCHER_SIGNATURE))?;
    let contract = IBatchSettlement::new(BATCH_SETTLEMENT_ADDRESS, fac.provider.inner());
    let ch = contract
        .channels(voucher.channel_id)
        .call()
        .await
        .map_err(|_| VerificationError::from_wire(RPC_READ_FAILED))?;
    if ch.balance == 0 {
        return Err(VerificationError::from_wire(CHANNEL_NOT_FOUND).into());
    }
    let max = voucher.max_claimable_amount.0;
    let balance = alloy_primitives::U256::from(ch.balance);
    let claimed = alloy_primitives::U256::from(ch.totalClaimed);
    if max > balance {
        return Err(VerificationError::from_wire(CUMULATIVE_EXCEEDS_BALANCE).into());
    }
    if max < claimed {
        return Err(VerificationError::from_wire(CUMULATIVE_BELOW_CLAIMED).into());
    }
    Ok(wire::VerifyResponse::valid(config.payer.to_string()))
}

pub(super) async fn execute_refund<P>(
    fac: &Eip155BatchSettlementFacilitator<P>,
    payload: &v2::PaymentPayload,
    requirements: &v2::PaymentRequirements,
) -> Result<wire::SettleResponse, FacilitatorError>
where
    P: Eip155MetaTransactionProvider + Sync,
    P::Inner: Provider,
    Eip155ExactError: From<P::Error>,
{
    let network: CompactString = requirements.network.to_string().into();
    let Some(parts) = refund_parts(&payload.payload) else {
        return Ok(settle_failure(INVALID_PAYLOAD_TYPE, None, "", network));
    };
    let payer = Some(parts.config.payer);
    if parts.refund_sig.is_none() {
        return Ok(settle_failure(
            AUTHORIZER_NOT_CONFIGURED,
            payer,
            "",
            network,
        ));
    }
    if !parts.claims.is_empty() && parts.claim_sig.is_none() {
        return Ok(settle_failure(
            AUTHORIZER_NOT_CONFIGURED,
            payer,
            "",
            network,
        ));
    }

    let contract = IBatchSettlement::new(BATCH_SETTLEMENT_ADDRESS, fac.provider.inner());
    let channel_id = payload
        .payload
        .voucher()
        .map(|v| v.channel_id)
        .unwrap_or_default();
    match contract.channels(channel_id).call().await {
        Ok(ch) if ch.balance == 0 => {
            return Ok(settle_failure(REFUND_NO_BALANCE, payer, "", network));
        }
        Ok(_) => {}
        Err(_) => return Ok(settle_failure(RPC_READ_FAILED, payer, "", network)),
    }

    let calldata = match refund_calldata(&parts) {
        Ok(c) => c,
        Err(e) => return facilitator_err_to_settle(e, payer, network),
    };
    simulate_and_broadcast(
        &fac.provider,
        Broadcast {
            calldata,
            sim_reason: REFUND_SIMULATION_FAILED,
            tx_reason: REFUND_TRANSACTION_FAILED,
            pending_key: None,
            pending: fac.pending.as_ref(),
            payer,
            network,
            amount: Some(parts.amount.to_string().into()),
        },
    )
    .await
}

struct RefundParts<'a> {
    config: &'a ChannelConfig,
    amount: alloy_primitives::U256,
    nonce: alloy_primitives::U256,
    refund_sig: Option<&'a Bytes>,
    claim_sig: Option<&'a Bytes>,
    claims: &'a [crate::batch_settlement::payload::VoucherClaim],
}

fn refund_parts(payload: &BatchSettlementPayload) -> Option<RefundParts<'_>> {
    match payload {
        BatchSettlementPayload::Refund {
            channel_config,
            amount: Some(amount),
            refund_nonce: Some(nonce),
            claims: Some(claims),
            refund_authorizer_signature,
            claim_authorizer_signature,
            ..
        } => Some(RefundParts {
            config: channel_config,
            amount: amount.0,
            nonce: nonce.0,
            refund_sig: refund_authorizer_signature.as_ref(),
            claim_sig: claim_authorizer_signature.as_ref(),
            claims,
        }),
        _ => None,
    }
}

fn refund_calldata(parts: &RefundParts<'_>) -> Result<Bytes, FacilitatorError> {
    let sol_cfg = to_sol_config(parts.config)?;
    let refund_sig = parts.refund_sig.cloned().unwrap_or_default();
    let refund = Bytes::from(
        IBatchSettlement::refundWithSignatureCall {
            config: sol_cfg,
            amount: u128_amount(parts.amount)?,
            nonce: parts.nonce,
            receiverAuthorizerSignature: refund_sig,
        }
        .abi_encode(),
    );
    if parts.claims.is_empty() {
        return Ok(refund);
    }
    let claim_sig = parts.claim_sig.cloned().unwrap_or_default();
    let claim = Bytes::from(
        IBatchSettlement::claimWithSignatureCall {
            voucherClaims: to_sol_claims(parts.claims)?,
            authorizerSignature: claim_sig,
        }
        .abi_encode(),
    );
    Ok(Bytes::from(
        IBatchSettlement::multicallCall {
            data: vec![claim, refund],
        }
        .abi_encode(),
    ))
}
