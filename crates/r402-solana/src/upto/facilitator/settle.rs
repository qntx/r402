//! Deposit (`open`) and claim (`settle_and_seal` + `distribute`) settlement.
#![allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    unreachable_pub,
    reason = "deposit/claim take the facilitator runtime pieces explicitly"
)]

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Mutex;

use r402_core::error::{ErrorReason, FacilitatorError};
use r402_core::pending_settlement::PendingSettlementStore;
use r402_core::wire::{Extensions, PaymentRequirements, SettleResponse};
use solana_client::rpc_config::RpcSimulateTransactionConfig;
use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::Instruction;
use solana_message::VersionedMessage;
use solana_message::v0::Message as MessageV0;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;

use super::config::SolanaUptoFacilitatorConfig;
use super::storage::{UptoChannelRecord, UptoChannelStorage};
use super::verify::{OpenAuth, UptoFailure, decode_transaction, unix_now};
use crate::chain::provider::{SolanaChainProviderError, SolanaChainProviderLike};

use crate::upto::error::codes;
use crate::upto::payment_channels::{
    ChannelStatus, DistributeInstructionArgs, build_distribute_instruction,
    build_ed25519_verify_instruction, build_settle_and_seal_instruction, decode_channel,
    encode_voucher_message, verify_voucher_signature,
};
use crate::upto::shared::{
    PaymentChannelConfig, resolve_payment_channel_config, resolve_token_program,
};
use crate::upto::types::UptoSvmPayload;

/// In-flight deposit/claim dedup keys (released on simulation failure).
#[derive(Debug, Default)]
pub struct InflightSettlements {
    keys: Mutex<HashSet<String>>,
}

impl InflightSettlements {
    /// Reserve `key`. Returns `true` when this caller owns it.
    pub fn reserve(&self, key: &str) -> bool {
        self.keys
            .lock()
            .is_ok_and(|mut guard| guard.insert(key.to_owned()))
    }

    /// Release `key` so a retry may proceed.
    pub fn release(&self, key: &str) {
        if let Ok(mut guard) = self.keys.lock() {
            guard.remove(key);
        }
    }
}

/// Deposit: validate, simulate, co-sign, broadcast `open`.
pub async fn settle_deposit<P: SolanaChainProviderLike>(
    provider: &P,
    payload: &UptoSvmPayload,
    requirements: &PaymentRequirements,
    auth: OpenAuth,
    _config: &SolanaUptoFacilitatorConfig,
    storage: &dyn UptoChannelStorage,
    pending: &dyn PendingSettlementStore,
    inflight: &InflightSettlements,
) -> Result<SettleResponse, FacilitatorError> {
    let network = requirements.network.to_string();
    let deposit_key = format!("upto:deposit:{network}:{}", payload.channel_id);
    if let Some(response) = reconcile_pending(
        provider,
        pending,
        &deposit_key,
        payload,
        &auth.max_amount.to_string(),
        &network,
    )
    .await?
    {
        return Ok(response);
    }
    let channel_pk = Pubkey::from_str(&payload.channel_id)
        .map_err(|err| failure(codes::CHANNEL_ID, payload, err.to_string()).into_error())?;
    if channel_exists(provider, &channel_pk).await? {
        return Ok(failure(
            codes::CHANNEL_ALREADY_OPEN,
            payload,
            "channel already exists",
        )
        .into_settle(&network));
    }
    if !inflight.reserve(&deposit_key) {
        return Ok(
            failure("duplicate_settlement", payload, "deposit already in flight")
                .into_settle(&network),
        );
    }
    if let Err(err) = simulate_open(provider, &auth.transaction).await {
        inflight.release(&deposit_key);
        return Ok(
            failure(codes::SETTLEMENT_SIMULATION, payload, err.to_string()).into_settle(&network),
        );
    }
    if let Err(err) = storage.upsert(channel_record(payload, requirements, &auth)) {
        inflight.release(&deposit_key);
        return Ok(
            failure(codes::CHANNEL_BROADCAST, payload, err.to_string()).into_settle(&network)
        );
    }
    match broadcast_open(provider, auth.transaction).await {
        Ok(signature) => {
            pending.delete(&deposit_key);
            Ok(success(
                signature,
                &network,
                &auth.max_amount.to_string(),
                &payload.from,
            ))
        }
        Err((Some(signature), err)) => {
            pending.set(&deposit_key, signature.to_string().into());
            Ok(pending_failure(
                &signature.to_string(),
                &payload.from,
                &network,
                &err.to_string(),
            ))
        }
        Err((None, err)) => {
            inflight.release(&deposit_key);
            Ok(failure(codes::CHANNEL_BROADCAST, payload, err.to_string()).into_settle(&network))
        }
    }
}

/// Claim: verify voucher, then `settle_and_seal` + `distribute`.
pub async fn settle_claim<P: SolanaChainProviderLike>(
    provider: &P,
    payload: &UptoSvmPayload,
    requirements: &PaymentRequirements,
    actual: u64,
    config: &SolanaUptoFacilitatorConfig,
    storage: &dyn UptoChannelStorage,
    pending: &dyn PendingSettlementStore,
    inflight: &InflightSettlements,
) -> Result<SettleResponse, FacilitatorError> {
    let network = requirements.network.to_string();
    let claim_key = format!("upto:{network}:{}", payload.channel_id);
    if let Some(response) = reconcile_pending(
        provider,
        pending,
        &claim_key,
        payload,
        &actual.to_string(),
        &network,
    )
    .await?
    {
        return Ok(response);
    }
    let voucher = payload
        .voucher_signature
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| failure(codes::MISSING_VOUCHER, payload, "missing voucher").into_error())?;
    let channel = resolve_payment_channel_config(requirements).map_err(|err| {
        failure(codes::PAYMENT_REQUIREMENTS, payload, err.to_string()).into_error()
    })?;
    if payload.authorized_signer != channel.receiver_authorizer {
        return Ok(
            failure(codes::RECEIVER_AUTHORIZER, payload, "authorizer mismatch")
                .into_settle(&network),
        );
    }
    let fee_payer = Pubkey::from_str(&channel.fee_payer).map_err(|err| {
        failure(codes::PAYMENT_REQUIREMENTS, payload, err.to_string()).into_error()
    })?;
    if fee_payer != provider.pubkey() {
        return Ok(
            failure("facilitator_mismatch", payload, "feePayer mismatch").into_settle(&network),
        );
    }
    check_claim_window(payload)?;
    let signature = Signature::from_str(voucher)
        .map_err(|err| failure(codes::VOUCHER_SIGNATURE, payload, err.to_string()).into_error())?;
    let channel_pk = Pubkey::from_str(&payload.channel_id)
        .map_err(|err| failure(codes::CHANNEL_ID, payload, err.to_string()).into_error())?;
    let authorizer = Pubkey::from_str(&payload.authorized_signer).map_err(|err| {
        failure(codes::RECEIVER_AUTHORIZER, payload, err.to_string()).into_error()
    })?;
    let message = encode_voucher_message(&channel_pk, actual, payload.expires_at);
    if !verify_voucher_signature(&signature, &authorizer, &message) {
        return Ok(failure(
            codes::VOUCHER_SIGNATURE,
            payload,
            "voucher not signed by authorizer",
        )
        .into_settle(&network));
    }
    let token_program = resolve_token_program(requirements).map_err(|err| {
        failure(codes::PAYMENT_REQUIREMENTS, payload, err.to_string()).into_error()
    })?;
    let mint = Pubkey::from_str(requirements.asset.as_str()).map_err(|err| {
        failure(codes::PAYMENT_REQUIREMENTS, payload, err.to_string()).into_error()
    })?;
    let from = Pubkey::from_str(&payload.from)
        .map_err(|err| failure(codes::PAYER_MISMATCH, payload, err.to_string()).into_error())?;
    verify_onchain_channel(
        provider,
        &channel_pk,
        &channel,
        actual,
        payload,
        &from,
        &mint,
    )
    .await?;
    if !inflight.reserve(&claim_key) {
        return Ok(
            failure("duplicate_settlement", payload, "claim already in flight")
                .into_settle(&network),
        );
    }
    let ixs = claim_instructions(
        &channel_pk,
        fee_payer,
        from,
        mint,
        token_program,
        &channel,
        actual,
        &signature,
        &authorizer,
        payload.expires_at,
        &network,
        config,
    )?;
    match submit_claim(provider, fee_payer, ixs).await {
        Ok(tx_sig) => {
            drop(storage.upsert(channel_record(
                payload,
                requirements,
                &OpenAuth {
                    payload: payload.clone(),
                    channel,
                    max_amount: actual,
                    token_program,
                    transaction: decode_transaction(&payload.open_transaction).unwrap_or_else(
                        |_| VersionedTransaction {
                            signatures: vec![],
                            message: VersionedMessage::Legacy(solana_message::Message::default()),
                        },
                    ),
                },
            )));
            pending.delete(&claim_key);
            Ok(success(
                tx_sig,
                &network,
                &actual.to_string(),
                &payload.from,
            ))
        }
        Err((Some(tx_sig), err)) => {
            pending.set(&claim_key, tx_sig.to_string().into());
            Ok(pending_failure(
                &tx_sig.to_string(),
                &payload.from,
                &network,
                &err.to_string(),
            ))
        }
        Err((None, err)) => {
            inflight.release(&claim_key);
            Ok(failure("transaction_failed", payload, err.to_string()).into_settle(&network))
        }
    }
}

fn claim_instructions(
    channel: &Pubkey,
    payee: Pubkey,
    payer: Pubkey,
    mint: Pubkey,
    token_program: Pubkey,
    channel_cfg: &PaymentChannelConfig,
    actual: u64,
    voucher_sig: &Signature,
    authorizer: &Pubkey,
    expires_at: i64,
    network: &str,
    config: &SolanaUptoFacilitatorConfig,
) -> Result<Vec<Instruction>, FacilitatorError> {
    let mut ixs = Vec::new();
    if config.settle_compute_unit_limit > 0 {
        ixs.push(ComputeBudgetInstruction::set_compute_unit_limit(
            config.settle_compute_unit_limit,
        ));
    }
    if config.compute_unit_price_micro_lamports > 0 {
        ixs.push(ComputeBudgetInstruction::set_compute_unit_price(
            config.compute_unit_price_micro_lamports,
        ));
    }
    if actual > 0 {
        let message = encode_voucher_message(channel, actual, expires_at);
        let sig_bytes: [u8; 64] = <[u8; 64]>::try_from(voucher_sig.as_ref()).map_err(|_| {
            FacilitatorError::Verification(r402_core::error::VerificationError::InvalidSignature(
                "voucher signature is not 64 bytes".into(),
            ))
        })?;
        ixs.push(build_ed25519_verify_instruction(
            &message, &sig_bytes, authorizer,
        ));
    }
    ixs.push(build_settle_and_seal_instruction(
        *channel,
        payee,
        actual > 0,
    ));
    ixs.push(
        build_distribute_instruction(&DistributeInstructionArgs {
            channel: *channel,
            payer,
            payee,
            rent_payer: payee,
            mint,
            token_program,
            splits: channel_cfg.splits.clone(),
            network: network.to_owned(),
        })
        .map_err(|err| FacilitatorError::Onchain(err.to_string()))?,
    );
    Ok(ixs)
}

async fn submit_claim<P: SolanaChainProviderLike>(
    provider: &P,
    fee_payer: Pubkey,
    ixs: Vec<Instruction>,
) -> Result<Signature, (Option<Signature>, SolanaChainProviderError)> {
    let rpc = provider.rpc_client();
    let (blockhash, _) = rpc
        .get_latest_blockhash_with_commitment(CommitmentConfig {
            commitment: CommitmentLevel::Confirmed,
        })
        .await
        .map_err(|err| (None, SolanaChainProviderError::from(err)))?;
    let message = MessageV0::try_compile(&fee_payer, &ixs, &[], blockhash)
        .map_err(|err| (None, SolanaChainProviderError::Custom(err.to_string())))?;
    let tx = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(message),
    };
    let signed = provider.sign(tx).map_err(|err| (None, err))?;
    match provider.send(&signed).await {
        Ok(signature) => match provider
            .send_and_confirm(
                &signed,
                CommitmentConfig {
                    commitment: CommitmentLevel::Confirmed,
                },
            )
            .await
        {
            Ok(confirmed) => Ok(confirmed),
            Err(err) => Err((Some(signature), err)),
        },
        Err(err) => Err((None, err)),
    }
}

async fn broadcast_open<P: SolanaChainProviderLike>(
    provider: &P,
    transaction: VersionedTransaction,
) -> Result<Signature, (Option<Signature>, SolanaChainProviderError)> {
    let signed = provider.sign(transaction).map_err(|err| (None, err))?;
    match provider.send(&signed).await {
        Ok(signature) => match provider
            .send_and_confirm(
                &signed,
                CommitmentConfig {
                    commitment: CommitmentLevel::Confirmed,
                },
            )
            .await
        {
            Ok(confirmed) => Ok(confirmed),
            Err(err) => Err((Some(signature), err)),
        },
        Err(err) => Err((None, err)),
    }
}

async fn simulate_open<P: SolanaChainProviderLike>(
    provider: &P,
    tx: &VersionedTransaction,
) -> Result<(), SolanaChainProviderError> {
    provider
        .simulate_transaction_with_config(
            tx,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: true,
                ..RpcSimulateTransactionConfig::default()
            },
        )
        .await
}

async fn channel_exists<P: SolanaChainProviderLike>(
    provider: &P,
    channel: &Pubkey,
) -> Result<bool, FacilitatorError> {
    let accounts = provider
        .get_multiple_accounts(&[*channel])
        .await
        .map_err(|err| FacilitatorError::Onchain(err.to_string()))?;
    Ok(accounts.first().is_some_and(Option::is_some))
}

async fn verify_onchain_channel<P: SolanaChainProviderLike>(
    provider: &P,
    channel_pk: &Pubkey,
    channel: &PaymentChannelConfig,
    _actual: u64,
    payload: &UptoSvmPayload,
    from: &Pubkey,
    mint: &Pubkey,
) -> Result<(), FacilitatorError> {
    let accounts = provider
        .get_multiple_accounts(&[*channel_pk])
        .await
        .map_err(|err| FacilitatorError::Onchain(err.to_string()))?;
    let account = accounts.first().and_then(Option::as_ref).ok_or_else(|| {
        failure(codes::CHANNEL_STATE, payload, "channel account missing").into_error()
    })?;
    let decoded = decode_channel(&account.data)
        .map_err(|err| failure(codes::CHANNEL_STATE, payload, err.to_string()).into_error())?;
    if decoded.status != ChannelStatus::Open {
        return Err(failure(codes::CHANNEL_STATE, payload, "channel is not open").into_error());
    }
    let payee = Pubkey::from_str(&channel.fee_payer).map_err(|err| {
        failure(codes::PAYMENT_REQUIREMENTS, payload, err.to_string()).into_error()
    })?;
    let authorizer = Pubkey::from_str(&channel.receiver_authorizer).map_err(|err| {
        failure(codes::RECEIVER_AUTHORIZER, payload, err.to_string()).into_error()
    })?;
    if decoded.payer != *from
        || decoded.payee != payee
        || decoded.authorized_signer != authorizer
        || decoded.mint != *mint
        || decoded.grace_period != channel.withdraw_delay
    {
        return Err(failure(codes::CHANNEL_STATE, payload, "channel fields mismatch").into_error());
    }
    Ok(())
}

async fn reconcile_pending<P: SolanaChainProviderLike>(
    provider: &P,
    pending: &dyn PendingSettlementStore,
    key: &str,
    payload: &UptoSvmPayload,
    amount: &str,
    network: &str,
) -> Result<Option<SettleResponse>, FacilitatorError> {
    let Some(signature) = pending.get(key) else {
        return Ok(None);
    };
    pending.delete(key);
    let parsed: Signature = signature.parse().map_err(|_| {
        failure(
            codes::CHANNEL_BROADCAST,
            payload,
            "malformed pending signature",
        )
        .into_error()
    })?;
    let rpc = provider.rpc_client();
    match rpc
        .confirm_transaction_with_commitment(
            &parsed,
            CommitmentConfig {
                commitment: CommitmentLevel::Confirmed,
            },
        )
        .await
    {
        Ok(status) if status.value => Ok(Some(success(parsed, network, amount, &payload.from))),
        Ok(_) | Err(_) => {
            pending.set(key, signature.clone());
            Ok(Some(pending_failure(
                signature.as_str(),
                &payload.from,
                network,
                "settlement still pending",
            )))
        }
    }
}

fn check_claim_window(payload: &UptoSvmPayload) -> Result<(), FacilitatorError> {
    let now = unix_now().map_err(|err| failure(codes::EXPIRED, payload, err).into_error())?;
    if now < payload.valid_after {
        return Err(failure(codes::NOT_YET_ACTIVE, payload, "not yet active").into_error());
    }
    if payload.expires_at == 0 || now >= payload.expires_at {
        return Err(failure(codes::EXPIRED, payload, "expired").into_error());
    }
    Ok(())
}

fn channel_record(
    payload: &UptoSvmPayload,
    requirements: &PaymentRequirements,
    auth: &OpenAuth,
) -> UptoChannelRecord {
    UptoChannelRecord {
        channel_id: payload.channel_id.clone(),
        network: requirements.network.to_string(),
        pay_to: requirements.pay_to.to_string(),
        token_program: auth.token_program.to_string(),
        expires_at: payload.expires_at,
    }
}

fn success(signature: Signature, network: &str, amount: &str, payer: &str) -> SettleResponse {
    SettleResponse::Success {
        payer: payer.into(),
        transaction: signature.to_string().into(),
        network: network.into(),
        amount: Some(amount.into()),
        extensions: Extensions::new(),
        extra: None,
    }
}

fn pending_failure(signature: &str, payer: &str, network: &str, message: &str) -> SettleResponse {
    SettleResponse::Failure {
        reason: ErrorReason::SettlementPending,
        message: Some(message.into()),
        payer: Some(payer.into()),
        transaction: signature.into(),
        network: network.into(),
        extensions: Extensions::new(),
        extra: None,
    }
}

fn failure(
    code: &'static str,
    payload: &UptoSvmPayload,
    message: impl Into<String>,
) -> UptoFailure {
    UptoFailure::new(code, payload.from.clone(), message)
}
