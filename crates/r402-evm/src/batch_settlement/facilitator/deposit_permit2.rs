//! Permit2 deposit authorization field checks and collector data.

use alloy_primitives::{Address, Bytes, U256};
use r402_protocol::error::VerificationError;
use r402_protocol::payment::UnixTimestamp;

use super::encoding::permit2_collector_data;
use crate::batch_settlement::errors::{
    PERMIT2_AMOUNT_MISMATCH, PERMIT2_AUTHORIZATION_REQUIRED, PERMIT2_DEADLINE_EXPIRED,
    PERMIT2_INVALID_SPENDER,
};
use crate::batch_settlement::payload::{
    ChannelConfig, DepositAuthorization, PERMIT2_DEPOSIT_COLLECTOR_ADDRESS,
};
use crate::signature::StructuredSignature;

pub(super) fn collector_data(auth: &DepositAuthorization) -> Result<Bytes, VerificationError> {
    let DepositAuthorization::Permit2 {
        permit2_authorization: permit2,
    } = auth
    else {
        return Err(VerificationError::from_wire(PERMIT2_AUTHORIZATION_REQUIRED));
    };
    let inner = match StructuredSignature::try_from_bytes(permit2.signature.clone())
        .map_err(|e| VerificationError::InvalidSignature(e.to_string()))?
    {
        StructuredSignature::EIP6492 { inner, .. } => inner,
        StructuredSignature::Unclassified(bytes) => bytes,
    };
    Ok(permit2_collector_data(
        permit2.nonce.0,
        permit2.deadline.0,
        inner,
        Bytes::new(),
    ))
}

pub(super) fn verify_permit2_fields(
    config: &ChannelConfig,
    deposit_amount: U256,
    asset: Address,
    auth: &DepositAuthorization,
) -> Result<(), VerificationError> {
    let DepositAuthorization::Permit2 {
        permit2_authorization: permit2,
    } = auth
    else {
        return Err(VerificationError::from_wire(PERMIT2_AUTHORIZATION_REQUIRED));
    };
    if permit2.spender != PERMIT2_DEPOSIT_COLLECTOR_ADDRESS {
        return Err(VerificationError::from_wire(PERMIT2_INVALID_SPENDER));
    }
    if permit2.permitted.token != config.token || permit2.permitted.token != asset {
        return Err(VerificationError::from_wire(PERMIT2_AMOUNT_MISMATCH));
    }
    if permit2.permitted.amount.0 != deposit_amount {
        return Err(VerificationError::from_wire(PERMIT2_AMOUNT_MISMATCH));
    }
    if permit2.from != config.payer {
        return Err(VerificationError::from_wire(PERMIT2_AUTHORIZATION_REQUIRED));
    }
    let now = U256::from(UnixTimestamp::now().as_secs());
    if permit2.deadline.0 < now {
        return Err(VerificationError::from_wire(PERMIT2_DEADLINE_EXPIRED));
    }
    Ok(())
}
