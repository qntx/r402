//! Claim settlement: `settle_and_seal` + `distribute`.
#![allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    unreachable_pub,
    reason = "claim takes facilitator runtime pieces explicitly"
)]

use std::str::FromStr;

use r402_facilitator::PendingSettlementStore;
use r402_protocol::{FacilitatorError, PaymentRequirements, SettleResponse};
use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_message::VersionedMessage;
use solana_message::v0::Message as MessageV0;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::Instruction;
use solana_transaction::versioned::VersionedTransaction;

use super::super::config::SolanaUptoFacilitatorConfig;
use super::super::storage::UptoChannelStorage;
use super::super::verify::{OpenAuth, decode_transaction, unix_now};
use super::{
    BroadcastFailure, InflightSettlements, channel_record, failure, pending_failure,
    reconcile_pending, success,
};
use crate::chain::provider::{SolanaChainProviderError, SolanaChainProviderLike};
use crate::upto::channel::{
    ChannelConfig, ChannelStatus, DistributeInstructionArgs, build_distribute_instruction,
    build_ed25519_verify_instruction, build_settle_and_seal_instruction, decode_channel,
    encode_voucher_message, resolve_payment_channel_config, resolve_token_program,
    verify_voucher_signature,
};
use crate::upto::error::codes;
use crate::upto::payload::UptoSvmPayload;

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
        Err(BroadcastFailure {
            signature: Some(tx_sig),
            error,
        }) => {
            pending.set(&claim_key, tx_sig.to_string().into());
            Ok(pending_failure(
                &tx_sig.to_string(),
                &payload.from,
                &network,
                &error.to_string(),
            ))
        }
        Err(BroadcastFailure {
            signature: None,
            error,
        }) => {
            inflight.release(&claim_key);
            Ok(failure("transaction_failed", payload, error.to_string()).into_settle(&network))
        }
    }
}

fn claim_instructions(
    channel: &Pubkey,
    payee: Pubkey,
    payer: Pubkey,
    mint: Pubkey,
    token_program: Pubkey,
    channel_cfg: &ChannelConfig,
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
            FacilitatorError::Verification(r402_protocol::VerificationError::InvalidSignature(
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
) -> Result<Signature, BroadcastFailure> {
    let rpc = provider.rpc_client();
    let (blockhash, _) = rpc
        .get_latest_blockhash_with_commitment(CommitmentConfig {
            commitment: CommitmentLevel::Confirmed,
        })
        .await
        .map_err(|err| BroadcastFailure {
            signature: None,
            error: Box::new(SolanaChainProviderError::from(err)),
        })?;
    let message = MessageV0::try_compile(&fee_payer, &ixs, &[], blockhash).map_err(|err| {
        BroadcastFailure {
            signature: None,
            error: Box::new(SolanaChainProviderError::Custom(err.to_string())),
        }
    })?;
    let tx = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(message),
    };
    let signed = provider.sign(tx).map_err(|err| BroadcastFailure {
        signature: None,
        error: Box::new(err),
    })?;
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
            Err(error) => Err(BroadcastFailure {
                signature: Some(signature),
                error: Box::new(error),
            }),
        },
        Err(error) => Err(BroadcastFailure {
            signature: None,
            error: Box::new(error),
        }),
    }
}

async fn verify_onchain_channel<P: SolanaChainProviderLike>(
    provider: &P,
    channel_pk: &Pubkey,
    channel: &ChannelConfig,
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
