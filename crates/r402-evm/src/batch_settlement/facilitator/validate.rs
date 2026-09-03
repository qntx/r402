//! Official `validateChannelConfig` wire reasons.

use alloy_primitives::Address;
use r402_protocol::error::VerificationError;

use crate::batch_settlement::channel::{compute_channel_id, withdraw_delay_valid};
use crate::batch_settlement::errors::{
    CHANNEL_ID_MISMATCH, RECEIVER_AUTHORIZER_MISMATCH, RECEIVER_MISMATCH, TOKEN_MISMATCH,
    WITHDRAW_DELAY_MISMATCH, WITHDRAW_DELAY_OUT_OF_RANGE,
};
use crate::batch_settlement::payload::{ChannelConfig, v2};

pub(super) fn validate_channel_config(
    config: &ChannelConfig,
    channel_id: alloy_primitives::B256,
    requirements: &v2::PaymentRequirements,
    chain_id: u64,
) -> Result<(), VerificationError> {
    match compute_channel_id(config, chain_id) {
        Err(_) => return Err(VerificationError::from_wire(WITHDRAW_DELAY_OUT_OF_RANGE)),
        Ok(expected) if expected != channel_id => {
            return Err(VerificationError::from_wire(CHANNEL_ID_MISMATCH));
        }
        Ok(_) => {}
    }
    if config.receiver != requirements.pay_to.0 {
        return Err(VerificationError::from_wire(RECEIVER_MISMATCH));
    }
    let extra = requirements
        .extra
        .as_ref()
        .ok_or_else(|| VerificationError::from_wire(RECEIVER_AUTHORIZER_MISMATCH))?;
    if extra.receiver_authorizer.0 == Address::ZERO
        || config.receiver_authorizer != extra.receiver_authorizer.0
    {
        return Err(VerificationError::from_wire(RECEIVER_AUTHORIZER_MISMATCH));
    }
    if config.token != requirements.asset.0 {
        return Err(VerificationError::from_wire(TOKEN_MISMATCH));
    }
    if config.withdraw_delay != extra.withdraw_delay {
        return Err(VerificationError::from_wire(WITHDRAW_DELAY_MISMATCH));
    }
    if !withdraw_delay_valid(config.withdraw_delay) {
        return Err(VerificationError::from_wire(WITHDRAW_DELAY_OUT_OF_RANGE));
    }
    Ok(())
}
