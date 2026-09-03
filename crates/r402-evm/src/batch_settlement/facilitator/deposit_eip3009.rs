//! ERC-3009 deposit authorization, collector data, and EIP-6492 allowlist.

use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::Provider;
use alloy_sol_types::{SolStruct, eip712_domain, sol};
use r402_protocol::error::VerificationError;
use r402_protocol::payment::UnixTimestamp;

use super::encoding::{erc3009_collector_data, erc3009_deposit_nonce};
use super::sigcheck::verify_hash_signature;
use crate::batch_settlement::errors::{
    EIP6492_FACTORY_NOT_ALLOWED, ERC3009_AUTHORIZATION_REQUIRED,
    INVALID_RECEIVE_AUTHORIZATION_SIGNATURE, MISSING_EIP712_DOMAIN, VALID_AFTER_IN_FUTURE,
    VALID_BEFORE_EXPIRED,
};
use crate::batch_settlement::payload::{
    ChannelConfig, DepositAuthorization, DepositErc3009Authorization,
    ERC3009_DEPOSIT_COLLECTOR_ADDRESS,
};
use crate::chain::Eip155ChainReference;
use crate::exact::facilitator::eip6492::factory_allowed;
use crate::signature::StructuredSignature;

sol! {
    struct ReceiveWithAuthorization {
        address from;
        address to;
        uint256 value;
        uint256 validAfter;
        uint256 validBefore;
        bytes32 nonce;
    }
}

pub(super) struct Counterfactual {
    pub factory: Address,
    pub factory_calldata: Bytes,
}

pub(super) fn collector_data(auth: &DepositErc3009Authorization, inner: Bytes) -> Bytes {
    erc3009_collector_data(auth.valid_after.0, auth.valid_before.0, auth.salt, inner)
}

#[allow(
    clippy::too_many_arguments,
    reason = "mirrors official verifyEip3009DepositAuthorization parameters"
)]
pub(super) async fn verify_eip3009_auth<P: Provider>(
    provider: &P,
    config: &ChannelConfig,
    voucher_channel_id: alloy_primitives::B256,
    deposit_amount: U256,
    auth: &DepositAuthorization,
    extra_name: &str,
    extra_version: &str,
    chain: &Eip155ChainReference,
    asset: Address,
    allowlist: &[Address],
) -> Result<(Bytes, Option<Counterfactual>), VerificationError> {
    let DepositAuthorization::Erc3009 {
        erc3009_authorization: erc3009,
    } = auth
    else {
        return Err(VerificationError::from_wire(ERC3009_AUTHORIZATION_REQUIRED));
    };
    if extra_name.is_empty() || extra_version.is_empty() {
        return Err(VerificationError::from_wire(MISSING_EIP712_DOMAIN));
    }
    check_time_window(erc3009.valid_after.0, erc3009.valid_before.0)?;

    let structured = StructuredSignature::try_from_bytes(erc3009.signature.clone())
        .map_err(|e| VerificationError::InvalidSignature(e.to_string()))?;
    match structured {
        StructuredSignature::EIP6492 {
            factory,
            factory_calldata,
            inner,
            ..
        } => {
            let code = provider.get_code_at(config.payer).await.unwrap_or_default();
            if code.is_empty() {
                if !factory_allowed(factory, allowlist) {
                    #[cfg(feature = "telemetry")]
                    tracing::warn!(factory = %factory, "invalid_batch_settlement_evm_eip6492_factory_not_allowed");
                    return Err(VerificationError::from_wire(EIP6492_FACTORY_NOT_ALLOWED));
                }
                return Ok((
                    inner,
                    Some(Counterfactual {
                        factory,
                        factory_calldata,
                    }),
                ));
            }
            verify_receive_auth(
                provider,
                config.payer,
                asset,
                extra_name,
                extra_version,
                *chain,
                deposit_amount,
                erc3009,
                voucher_channel_id,
                inner.clone(),
            )
            .await?;
            Ok((inner, None))
        }
        StructuredSignature::Unclassified(bytes) => {
            verify_receive_auth(
                provider,
                config.payer,
                asset,
                extra_name,
                extra_version,
                *chain,
                deposit_amount,
                erc3009,
                voucher_channel_id,
                bytes.clone(),
            )
            .await?;
            Ok((bytes, None))
        }
    }
}

fn check_time_window(valid_after: U256, valid_before: U256) -> Result<(), VerificationError> {
    let now = UnixTimestamp::now().as_secs();
    let after = u64::try_from(valid_after).unwrap_or(u64::MAX);
    let before = u64::try_from(valid_before).unwrap_or(0);
    if before < now.saturating_add(crate::EVM_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS) {
        return Err(VerificationError::from_wire(VALID_BEFORE_EXPIRED));
    }
    if after > now {
        return Err(VerificationError::from_wire(VALID_AFTER_IN_FUTURE));
    }
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "mirrors official verifyReceiveAuth parameters"
)]
async fn verify_receive_auth<P: Provider>(
    provider: &P,
    payer: Address,
    asset: Address,
    name: &str,
    version: &str,
    chain: Eip155ChainReference,
    amount: U256,
    auth: &DepositErc3009Authorization,
    channel_id: alloy_primitives::B256,
    signature: Bytes,
) -> Result<(), VerificationError> {
    let domain = eip712_domain! {
        name: name.to_owned(),
        version: version.to_owned(),
        chain_id: chain.inner(),
        verifying_contract: asset,
    };
    let typed = ReceiveWithAuthorization {
        from: payer,
        to: ERC3009_DEPOSIT_COLLECTOR_ADDRESS,
        value: amount,
        validAfter: auth.valid_after.0,
        validBefore: auth.valid_before.0,
        nonce: erc3009_deposit_nonce(channel_id, auth.salt),
    };
    let hash = typed.eip712_signing_hash(&domain);
    verify_hash_signature(
        provider,
        payer,
        hash,
        signature,
        INVALID_RECEIVE_AUTHORIZATION_SIGNATURE,
    )
    .await
}
