//! Off-chain verification for batch-settlement request payloads.

use alloy_primitives::U256;
use r402_core::error::VerificationError;
use r402_core::scheme::BatchSettlementScheme;

use super::channel::{channel_id_binding_error, withdraw_delay_valid};
use super::store::ChannelStore;
use super::types::{BatchSettlementPayload, v2};
use super::voucher::verify_voucher_signature;
use crate::chain::TokenAmount;

/// Verifies a batch-settlement payment payload (voucher/deposit/refund).
///
/// - Binds `channelId` to `channelConfig`
/// - Validates voucher ECDSA (or defers smart-wallet sigs)
/// - For voucher path: ensures ceiling covers prior charged + request amount
///
/// Returns the charge amount to apply on settle (requirements.amount for voucher).
///
/// # Errors
///
/// Structured verification failures.
pub fn verify_offchain(
    payload: &v2::PaymentPayload,
    requirements: &v2::PaymentRequirements,
    chain_id: u64,
    store: &dyn ChannelStore,
) -> Result<TokenAmount, VerificationError> {
    if requirements.scheme != BatchSettlementScheme {
        return Err(VerificationError::UnsupportedScheme);
    }
    if payload.accepted.scheme.to_string() != "batch-settlement" {
        return Err(VerificationError::AcceptedRequirementsMismatch);
    }
    if payload.accepted.network != requirements.network {
        return Err(VerificationError::ChainIdMismatch);
    }

    let extra = requirements
        .extra
        .as_ref()
        .ok_or_else(|| VerificationError::InvalidFormat("missing batch-settlement extra".into()))?;
    if !withdraw_delay_valid(extra.withdraw_delay) {
        return Err(VerificationError::InvalidFormat(
            "withdrawDelay out of range".into(),
        ));
    }

    let body = &payload.payload;
    let config = body.channel_config();
    let voucher = body.voucher();

    if let Some(err) = channel_id_binding_error(config, voucher.channel_id, chain_id) {
        return Err(VerificationError::InvalidFormat(err.into()));
    }

    if config.receiver != requirements.pay_to.0 {
        return Err(VerificationError::RecipientMismatch);
    }
    if config.token != requirements.asset.0 {
        return Err(VerificationError::AssetMismatch);
    }
    if config.receiver_authorizer != extra.receiver_authorizer.0 {
        return Err(VerificationError::InvalidFormat(
            "receiverAuthorizer mismatch".into(),
        ));
    }
    if config.withdraw_delay != extra.withdraw_delay {
        return Err(VerificationError::InvalidFormat(
            "withdrawDelay mismatch".into(),
        ));
    }

    verify_voucher_signature(voucher, chain_id, config.payer, config.payer_authorizer)?;

    let request_amount = requirements.amount;
    let state = store.get(&voucher.channel_id);

    match body {
        BatchSettlementPayload::Deposit { .. } => Err(VerificationError::InvalidFormat(
            "batch-settlement deposit settle requires on-chain deposit (not implemented on request path; use voucher after external funding)".into(),
        )),
        BatchSettlementPayload::Voucher { .. } => {
            let needed = state.charged_cumulative.0.saturating_add(request_amount.0);
            if voucher.max_claimable_amount.0 < needed {
                return Err(VerificationError::InvalidFormat(
                    "voucher maxClaimableAmount below charged+request".into(),
                ));
            }
            Ok(request_amount)
        }
        BatchSettlementPayload::Refund { amount, .. } => {
            // Zero-charge voucher: max should equal charged cumulative.
            if voucher.max_claimable_amount.0 != state.charged_cumulative.0 {
                return Err(VerificationError::InvalidFormat(
                    "refund voucher maxClaimableAmount must equal charged cumulative".into(),
                ));
            }
            Ok(amount.unwrap_or(TokenAmount::from(U256::ZERO)))
        }
    }
}
