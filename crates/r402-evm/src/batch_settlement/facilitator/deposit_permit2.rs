//! Permit2 deposit authorization: fields, typed-data `SignatureChecker`, allowance.

use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_provider::Provider;
use alloy_sol_types::{SolStruct, eip712_domain, sol};
use r402_protocol::error::VerificationError;
use r402_protocol::payment::UnixTimestamp;

use super::encoding::permit2_collector_data;
use super::sigcheck::verify_hash_signature;
use crate::batch_settlement::errors::{
    CHANNEL_ID_MISMATCH, PERMIT2_ALLOWANCE_REQUIRED, PERMIT2_AMOUNT_MISMATCH,
    PERMIT2_AUTHORIZATION_REQUIRED, PERMIT2_DEADLINE_EXPIRED, PERMIT2_INVALID_SIGNATURE,
    PERMIT2_INVALID_SPENDER, TOKEN_MISMATCH,
};
use crate::batch_settlement::payload::{
    ChannelConfig, DepositAuthorization, PERMIT2_DEPOSIT_COLLECTOR_ADDRESS,
};
use crate::chain::contracts::IERC20;
use crate::permit2::PERMIT2_ADDRESS;
use crate::signature::StructuredSignature;

sol! {
    struct TokenPermissions {
        address token;
        uint256 amount;
    }

    struct DepositWitness {
        bytes32 channelId;
    }

    struct PermitWitnessTransferFrom {
        TokenPermissions permitted;
        address spender;
        uint256 nonce;
        uint256 deadline;
        DepositWitness witness;
    }
}

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

pub(super) async fn verify_permit2_auth<P: Provider>(
    provider: &P,
    config: &ChannelConfig,
    voucher_channel_id: B256,
    deposit_amount: U256,
    asset: Address,
    chain_id: u64,
    auth: &DepositAuthorization,
) -> Result<(), VerificationError> {
    let DepositAuthorization::Permit2 {
        permit2_authorization: permit2,
    } = auth
    else {
        return Err(VerificationError::from_wire(PERMIT2_AUTHORIZATION_REQUIRED));
    };
    if permit2.from != config.payer {
        return Err(VerificationError::from_wire(PERMIT2_INVALID_SIGNATURE));
    }
    if permit2.spender != PERMIT2_DEPOSIT_COLLECTOR_ADDRESS {
        return Err(VerificationError::from_wire(PERMIT2_INVALID_SPENDER));
    }
    if permit2.permitted.token != asset {
        return Err(VerificationError::from_wire(TOKEN_MISMATCH));
    }
    if permit2.permitted.amount.0 != deposit_amount {
        return Err(VerificationError::from_wire(PERMIT2_AMOUNT_MISMATCH));
    }
    if permit2.witness.channel_id != voucher_channel_id {
        return Err(VerificationError::from_wire(CHANNEL_ID_MISMATCH));
    }
    let now = UnixTimestamp::now().as_secs();
    let deadline = u64::try_from(permit2.deadline.0).unwrap_or(0);
    if deadline < now.saturating_add(crate::EVM_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS) {
        return Err(VerificationError::from_wire(PERMIT2_DEADLINE_EXPIRED));
    }

    let domain = eip712_domain! {
        name: "Permit2",
        chain_id: chain_id,
        verifying_contract: PERMIT2_ADDRESS,
    };
    let typed = PermitWitnessTransferFrom {
        permitted: TokenPermissions {
            token: permit2.permitted.token,
            amount: permit2.permitted.amount.0,
        },
        spender: permit2.spender,
        nonce: permit2.nonce.0,
        deadline: permit2.deadline.0,
        witness: DepositWitness {
            channelId: permit2.witness.channel_id,
        },
    };
    verify_hash_signature(
        provider,
        permit2.from,
        typed.eip712_signing_hash(&domain),
        permit2.signature.clone(),
        PERMIT2_INVALID_SIGNATURE,
    )
    .await?;

    let token = IERC20::new(asset, provider);
    match token.allowance(config.payer, PERMIT2_ADDRESS).call().await {
        Ok(allowance) if allowance >= deposit_amount => Ok(()),
        Ok(_) | Err(_) => Err(VerificationError::from_wire(PERMIT2_ALLOWANCE_REQUIRED)),
    }
}
