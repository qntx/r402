//! Deposit verify + on-chain `deposit()` settle (pending store + 6492).

use alloy_network::TransactionBuilder;
use alloy_primitives::{Address, Bytes, TxHash, hex};
use alloy_provider::bindings::IMulticall3;
use alloy_provider::{MULTICALL3_ADDRESS, Provider};
use alloy_rpc_types_eth::TransactionRequest;
use alloy_sol_types::SolCall;
use compact_str::CompactString;
use r402_protocol::error::{FacilitatorError, VerificationError};
use r402_protocol::network::ChainProvider;
use r402_protocol::payment::{SettleResponse, VerifyResponse};

use super::Eip155BatchSettlementFacilitator;
use super::contract::IBatchSettlement;
use super::deposit_eip3009::{self, Counterfactual};
use super::deposit_permit2;
use super::encoding::{to_sol_config, u128_amount};
use super::response::{facilitator_err_to_settle, settle_failure, verify_err_to_settle};
use super::send::{Broadcast, simulate_and_broadcast};
use super::validate::validate_channel_config;
use crate::asset::AssetTransferMethod;
use crate::batch_settlement::errors::{
    CUMULATIVE_BELOW_CLAIMED, CUMULATIVE_EXCEEDS_BALANCE, DEPOSIT_SIMULATION_FAILED,
    DEPOSIT_TRANSACTION_FAILED, INSUFFICIENT_BALANCE, INVALID_PAYLOAD_TYPE,
    INVALID_VOUCHER_SIGNATURE, RPC_READ_FAILED, SMART_WALLET_DEPLOYMENT_FAILED,
};
use crate::batch_settlement::payload::{
    BatchSettlementPayload, ChannelConfig, DepositAuthorization, DepositBody,
    ERC3009_DEPOSIT_COLLECTOR_ADDRESS, PERMIT2_DEPOSIT_COLLECTOR_ADDRESS, v2,
};
use crate::batch_settlement::voucher::verify_voucher_signature;
use crate::chain::contracts::IERC20;
use crate::chain::{Eip155MetaTransactionProvider, MetaTransaction};
use crate::error::Eip155ExactError;

pub(super) async fn verify_deposit<P>(
    fac: &Eip155BatchSettlementFacilitator<P>,
    payload: &v2::PaymentPayload,
    requirements: &v2::PaymentRequirements,
    chain_id: u64,
) -> Result<VerifyResponse, FacilitatorError>
where
    P: Eip155MetaTransactionProvider + ChainProvider + Sync,
    P::Inner: Provider,
{
    let payer = prepare_deposit(fac, payload, requirements, chain_id).await?;
    Ok(VerifyResponse::valid(payer.to_string()))
}

pub(super) async fn settle_deposit<P>(
    fac: &Eip155BatchSettlementFacilitator<P>,
    payload: &v2::PaymentPayload,
    requirements: &v2::PaymentRequirements,
) -> Result<SettleResponse, FacilitatorError>
where
    P: Eip155MetaTransactionProvider + ChainProvider + Send + Sync,
    P::Inner: Provider,
    Eip155ExactError: From<P::Error>,
{
    let network: CompactString = requirements.network.to_string().into();
    let (config, deposit) = deposit_parts(payload)?;
    let payer = Some(config.payer);
    let pending_key = pending_key(&deposit.authorization);
    if let Some(cached) = fac.pending.get(&pending_key) {
        fac.pending.delete(&pending_key);
        return reconcile(
            fac,
            &pending_key,
            cached,
            payer,
            network,
            deposit.amount.0.to_string(),
        )
        .await;
    }

    let chain_id = fac.parse_chain_id()?;
    if let Err(e) = prepare_deposit(fac, payload, requirements, chain_id).await {
        return Ok(verify_err_to_settle(&e, payer, network));
    }

    let (collector, collector_data, counterfactual) =
        match resolve_execution(fac, payload, requirements, chain_id).await {
            Ok(v) => v,
            Err(e) => return facilitator_err_to_settle(e, payer, network),
        };

    if let Some(cf) = counterfactual
        && let Some(resp) = deploy_counterfactual(fac, &cf, payer, network.clone()).await?
    {
        return Ok(resp);
    }

    let calldata = match deposit_calldata(config, deposit.amount.0, collector, collector_data) {
        Ok(c) => c,
        Err(e) => return facilitator_err_to_settle(e, payer, network),
    };
    let key_owned = pending_key;
    simulate_and_broadcast(
        &fac.provider,
        Broadcast {
            calldata,
            sim_reason: DEPOSIT_SIMULATION_FAILED,
            tx_reason: DEPOSIT_TRANSACTION_FAILED,
            pending_key: Some(key_owned.as_str()),
            pending: fac.pending.as_ref(),
            payer,
            network,
            amount: Some(deposit.amount.0.to_string().into()),
        },
    )
    .await
}

fn deposit_parts(
    payload: &v2::PaymentPayload,
) -> Result<(&ChannelConfig, &DepositBody), VerificationError> {
    match &payload.payload {
        BatchSettlementPayload::Deposit {
            channel_config,
            deposit,
            ..
        } => Ok((channel_config, deposit)),
        _ => Err(VerificationError::from_wire(INVALID_PAYLOAD_TYPE)),
    }
}

fn pending_key(auth: &DepositAuthorization) -> String {
    match auth {
        DepositAuthorization::Erc3009 {
            erc3009_authorization,
        } => hex::encode_prefixed(&erc3009_authorization.signature),
        DepositAuthorization::Permit2 {
            permit2_authorization,
        } => hex::encode_prefixed(&permit2_authorization.signature),
    }
}

async fn reconcile<P>(
    fac: &Eip155BatchSettlementFacilitator<P>,
    key: &str,
    cached: CompactString,
    payer: Option<Address>,
    network: CompactString,
    amount: String,
) -> Result<SettleResponse, FacilitatorError>
where
    P: Eip155MetaTransactionProvider + Sync,
    P::Inner: Provider,
{
    let hash: TxHash = cached
        .parse()
        .map_err(|e| FacilitatorError::Onchain(format!("invalid pending settlement hash: {e}")))?;
    match fac.provider.inner().get_transaction_receipt(hash).await {
        Ok(Some(receipt)) if receipt.status() => {
            fac.pending.delete(key);
            Ok(super::response::settle_success(
                payer,
                receipt.transaction_hash,
                network,
                Some(amount.into()),
            ))
        }
        Ok(Some(receipt)) => {
            fac.pending.delete(key);
            Ok(settle_failure(
                DEPOSIT_TRANSACTION_FAILED,
                payer,
                receipt.transaction_hash.to_string(),
                network,
            ))
        }
        _ => {
            fac.pending.set(key, cached);
            Ok(settle_failure(
                r402_protocol::error::ErrorReason::SettlementPending.as_str(),
                payer,
                hash.to_string(),
                network,
            ))
        }
    }
}

async fn prepare_deposit<P>(
    fac: &Eip155BatchSettlementFacilitator<P>,
    payload: &v2::PaymentPayload,
    requirements: &v2::PaymentRequirements,
    chain_id: u64,
) -> Result<Address, VerificationError>
where
    P: Eip155MetaTransactionProvider + Sync,
    P::Inner: Provider,
{
    let (config, deposit) = deposit_parts(payload)?;
    let voucher = payload
        .payload
        .voucher()
        .ok_or_else(|| VerificationError::from_wire(INVALID_PAYLOAD_TYPE))?;
    validate_channel_config(config, voucher.channel_id, requirements, chain_id)?;
    verify_voucher_signature(voucher, chain_id, config.payer, config.payer_authorizer)
        .map_err(|_| VerificationError::from_wire(INVALID_VOUCHER_SIGNATURE))?;
    let (collector, collector_data, counterfactual) =
        resolve_execution(fac, payload, requirements, chain_id)
            .await
            .map_err(|e| match e {
                FacilitatorError::Verification(v) => v,
                other => VerificationError::InvalidFormat(other.to_string()),
            })?;
    read_and_check_state(fac, config, voucher, deposit.amount.0, requirements.asset.0).await?;
    let calldata = deposit_calldata(config, deposit.amount.0, collector, collector_data).map_err(
        |e| match e {
            FacilitatorError::Verification(v) => v,
            _other => VerificationError::from_wire(DEPOSIT_SIMULATION_FAILED),
        },
    )?;
    simulate_deposit(fac.provider.inner(), calldata, counterfactual.as_ref()).await?;
    Ok(config.payer)
}

async fn resolve_execution<P>(
    fac: &Eip155BatchSettlementFacilitator<P>,
    payload: &v2::PaymentPayload,
    requirements: &v2::PaymentRequirements,
    chain_id: u64,
) -> Result<(Address, Bytes, Option<Counterfactual>), FacilitatorError>
where
    P: Eip155MetaTransactionProvider + Sync,
    P::Inner: Provider,
{
    let extra = requirements.extra.as_ref().ok_or_else(|| {
        FacilitatorError::Verification(VerificationError::InvalidFormat(
            "missing batch-settlement extra".into(),
        ))
    })?;
    let (config, deposit) = deposit_parts(payload).map_err(FacilitatorError::Verification)?;
    let voucher = payload.payload.voucher().ok_or_else(|| {
        FacilitatorError::Verification(VerificationError::from_wire(INVALID_PAYLOAD_TYPE))
    })?;
    let method = extra
        .asset_transfer_method
        .unwrap_or(match &deposit.authorization {
            DepositAuthorization::Permit2 { .. } => AssetTransferMethod::Permit2,
            DepositAuthorization::Erc3009 { .. } => AssetTransferMethod::Eip3009,
        });
    match method {
        AssetTransferMethod::Permit2 => {
            deposit_permit2::verify_permit2_auth(
                fac.provider.inner(),
                config,
                voucher.channel_id,
                deposit.amount.0,
                requirements.asset.0,
                chain_id,
                &deposit.authorization,
            )
            .await
            .map_err(FacilitatorError::Verification)?;
            let data = deposit_permit2::collector_data(&deposit.authorization)
                .map_err(FacilitatorError::Verification)?;
            Ok((PERMIT2_DEPOSIT_COLLECTOR_ADDRESS, data, None))
        }
        AssetTransferMethod::Eip3009 => {
            let chain = fac.provider.chain();
            let (inner, cf) = deposit_eip3009::verify_eip3009_auth(
                fac.provider.inner(),
                config,
                voucher.channel_id,
                deposit.amount.0,
                &deposit.authorization,
                &extra.name,
                &extra.version,
                chain,
                requirements.asset.0,
                &fac.eip6492_allowed_factories,
            )
            .await
            .map_err(FacilitatorError::Verification)?;
            let DepositAuthorization::Erc3009 {
                erc3009_authorization,
            } = &deposit.authorization
            else {
                return Err(FacilitatorError::Verification(
                    VerificationError::from_wire(
                        crate::batch_settlement::errors::ERC3009_AUTHORIZATION_REQUIRED,
                    ),
                ));
            };
            Ok((
                ERC3009_DEPOSIT_COLLECTOR_ADDRESS,
                deposit_eip3009::collector_data(erc3009_authorization, inner),
                cf,
            ))
        }
    }
}

async fn read_and_check_state<P>(
    fac: &Eip155BatchSettlementFacilitator<P>,
    config: &ChannelConfig,
    voucher: &crate::batch_settlement::payload::VoucherFields,
    deposit_amount: alloy_primitives::U256,
    asset: Address,
) -> Result<(), VerificationError>
where
    P: Eip155MetaTransactionProvider + Sync,
    P::Inner: Provider,
{
    let contract = IBatchSettlement::new(
        crate::batch_settlement::payload::BATCH_SETTLEMENT_ADDRESS,
        fac.provider.inner(),
    );
    let token = IERC20::new(asset, fac.provider.inner());
    let ch = contract
        .channels(voucher.channel_id)
        .call()
        .await
        .map_err(|_| VerificationError::from_wire(RPC_READ_FAILED))?;
    let bal = token
        .balanceOf(config.payer)
        .call()
        .await
        .map_err(|_| VerificationError::from_wire(RPC_READ_FAILED))?;
    if bal < deposit_amount {
        return Err(VerificationError::from_wire(INSUFFICIENT_BALANCE));
    }
    let effective = alloy_primitives::U256::from(ch.balance).saturating_add(deposit_amount);
    if voucher.max_claimable_amount.0 > effective {
        return Err(VerificationError::from_wire(CUMULATIVE_EXCEEDS_BALANCE));
    }
    if voucher.max_claimable_amount.0 <= alloy_primitives::U256::from(ch.totalClaimed) {
        return Err(VerificationError::from_wire(CUMULATIVE_BELOW_CLAIMED));
    }
    Ok(())
}

async fn simulate_deposit<P: Provider>(
    provider: &P,
    calldata: Bytes,
    counterfactual: Option<&Counterfactual>,
) -> Result<(), VerificationError> {
    let (to, data) = match counterfactual {
        Some(cf) => {
            let aggregate = IMulticall3::aggregate3Call {
                calls: vec![
                    IMulticall3::Call3 {
                        allowFailure: true,
                        target: cf.factory,
                        callData: cf.factory_calldata.clone(),
                    },
                    IMulticall3::Call3 {
                        allowFailure: false,
                        target: crate::batch_settlement::payload::BATCH_SETTLEMENT_ADDRESS,
                        callData: calldata,
                    },
                ],
            };
            (MULTICALL3_ADDRESS, Bytes::from(aggregate.abi_encode()))
        }
        None => (
            crate::batch_settlement::payload::BATCH_SETTLEMENT_ADDRESS,
            calldata,
        ),
    };
    let req = TransactionRequest::default().with_to(to).with_input(data);
    if provider.call(req).await.is_err() {
        return Err(VerificationError::from_wire(DEPOSIT_SIMULATION_FAILED));
    }
    Ok(())
}

fn deposit_calldata(
    config: &ChannelConfig,
    amount: alloy_primitives::U256,
    collector: Address,
    collector_data: Bytes,
) -> Result<Bytes, FacilitatorError> {
    let sol_cfg = to_sol_config(config)?;
    Ok(Bytes::from(
        IBatchSettlement::depositCall {
            config: sol_cfg,
            amount: u128_amount(amount)?,
            collector,
            collectorData: collector_data,
        }
        .abi_encode(),
    ))
}

async fn deploy_counterfactual<P>(
    fac: &Eip155BatchSettlementFacilitator<P>,
    cf: &Counterfactual,
    payer: Option<Address>,
    network: CompactString,
) -> Result<Option<SettleResponse>, FacilitatorError>
where
    P: Eip155MetaTransactionProvider + Sync,
    P::Inner: Provider,
    Eip155ExactError: From<P::Error>,
{
    match Eip155MetaTransactionProvider::send_transaction(
        &fac.provider,
        MetaTransaction {
            to: cf.factory,
            calldata: cf.factory_calldata.clone(),
            confirmations: 1,
            from: None,
        },
    )
    .await
    {
        Ok(receipt) if receipt.status() => Ok(None),
        Ok(_) | Err(_) => Ok(Some(settle_failure(
            SMART_WALLET_DEPLOYMENT_FAILED,
            payer,
            "",
            network,
        ))),
    }
}
