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

use r402_facilitator::PendingSettlementStore;
use r402_protocol::{
    ErrorReason, Extensions, FacilitatorError, PaymentRequirements, SettleResponse,
};
use solana_client::rpc_config::RpcSimulateTransactionConfig;
use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;

use super::config::SolanaUptoFacilitatorConfig;
use super::storage::{UptoChannelRecord, UptoChannelStorage};
use super::verify::{OpenAuth, UptoFailure};
use crate::chain::provider::{SolanaChainProviderError, SolanaChainProviderLike};
use crate::exact::facilitator::transaction_message_hash;
use crate::upto::error::codes;
use crate::upto::payload::UptoSvmPayload;

/// Inflight / settlement-cache key: channel-scoped so concurrent differently-signed
/// opens for the same channel still collide.
fn deposit_channel_key(network: &str, channel_id: &str) -> String {
    format!("upto:deposit:{network}:{channel_id}")
}

/// Pending-store key: open-tx message hash so a differently-shaped retry does
/// not inherit a stale signature (official `upto:deposit:{network}:{hash}`).
fn deposit_pending_key(network: &str, open_tx: &VersionedTransaction) -> String {
    format!(
        "upto:deposit:{network}:{}",
        transaction_message_hash(open_tx)
    )
}

pub(super) struct BroadcastFailure {
    pub(super) signature: Option<Signature>,
    pub(super) error: Box<SolanaChainProviderError>,
}

mod distribute;
pub use distribute::settle_claim;

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
    let channel_key = deposit_channel_key(&network, &payload.channel_id);
    let pending_key = deposit_pending_key(&network, &auth.transaction);
    if let Some(response) = reconcile_pending(
        provider,
        pending,
        &pending_key,
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
    if !inflight.reserve(&channel_key) {
        return Ok(
            failure("duplicate_settlement", payload, "deposit already in flight")
                .into_settle(&network),
        );
    }
    if let Err(err) = simulate_open(provider, &auth.transaction).await {
        inflight.release(&channel_key);
        return Ok(
            failure(codes::SETTLEMENT_SIMULATION, payload, err.to_string()).into_settle(&network),
        );
    }
    if let Err(err) = storage.upsert(channel_record(payload, requirements, &auth)) {
        inflight.release(&channel_key);
        return Ok(
            failure(codes::CHANNEL_BROADCAST, payload, err.to_string()).into_settle(&network)
        );
    }
    match broadcast_open(provider, auth.transaction).await {
        Ok(signature) => {
            pending.delete(&pending_key);
            Ok(success(
                signature,
                &network,
                &auth.max_amount.to_string(),
                &payload.from,
            ))
        }
        Err(BroadcastFailure {
            signature: Some(signature),
            error,
        }) => {
            pending.set(&pending_key, signature.to_string().into());
            Ok(pending_failure(
                &signature.to_string(),
                &payload.from,
                &network,
                &error.to_string(),
            ))
        }
        Err(BroadcastFailure {
            signature: None,
            error,
        }) => {
            inflight.release(&channel_key);
            Ok(failure(codes::CHANNEL_BROADCAST, payload, error.to_string()).into_settle(&network))
        }
    }
}

pub(super) async fn broadcast_open<P: SolanaChainProviderLike>(
    provider: &P,
    transaction: VersionedTransaction,
) -> Result<Signature, BroadcastFailure> {
    let signed = provider
        .sign(transaction)
        .map_err(|error| BroadcastFailure {
            signature: None,
            error: Box::new(error),
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

pub(super) async fn simulate_open<P: SolanaChainProviderLike>(
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
        .map(|_| ())
}

pub(super) async fn channel_exists<P: SolanaChainProviderLike>(
    provider: &P,
    channel: &Pubkey,
) -> Result<bool, FacilitatorError> {
    let accounts = provider
        .get_multiple_accounts(&[*channel])
        .await
        .map_err(|err| FacilitatorError::Onchain(err.to_string()))?;
    Ok(accounts.first().is_some_and(Option::is_some))
}

pub(super) async fn reconcile_pending<P: SolanaChainProviderLike>(
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

pub(super) fn channel_record(
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

pub(super) fn success(
    signature: Signature,
    network: &str,
    amount: &str,
    payer: &str,
) -> SettleResponse {
    SettleResponse::Success {
        payer: Some(payer.into()),
        transaction: signature.to_string().into(),
        network: network.into(),
        amount: Some(amount.into()),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    }
}

pub(super) fn pending_failure(
    signature: &str,
    payer: &str,
    network: &str,
    message: &str,
) -> SettleResponse {
    SettleResponse::Failure {
        reason: ErrorReason::SettlementPending,
        message: Some(message.into()),
        payer: Some(payer.into()),
        transaction: signature.into(),
        network: network.into(),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    }
}

pub(super) fn failure(
    code: &'static str,
    payload: &UptoSvmPayload,
    message: impl Into<String>,
) -> UptoFailure {
    UptoFailure::new(code, payload.from.clone(), message)
}

#[cfg(test)]
mod tests {
    use solana_message::{Message, VersionedMessage};
    use solana_pubkey::Pubkey;
    use solana_signature::Signature;
    use solana_transaction::Instruction;

    use super::*;

    fn dummy_open(marker: u8) -> VersionedTransaction {
        let payer = Pubkey::new_from_array([1u8; 32]);
        VersionedTransaction {
            signatures: vec![Signature::from([marker; 64])],
            message: VersionedMessage::Legacy(Message::new(
                &[Instruction {
                    program_id: Pubkey::new_from_array([marker; 32]),
                    accounts: Vec::new(),
                    data: vec![marker],
                }],
                Some(&payer),
            )),
        }
    }

    #[test]
    fn deposit_pending_key_is_open_tx_message_hash() {
        let network = "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1";
        let channel = "11111111111111111111111111111111";
        let a = dummy_open(1);
        let mut b = a.clone();
        b.signatures[0] = Signature::from([9u8; 64]);
        assert_eq!(
            deposit_pending_key(network, &a),
            deposit_pending_key(network, &b)
        );
        let c = dummy_open(2);
        assert_ne!(
            deposit_pending_key(network, &a),
            deposit_pending_key(network, &c)
        );
        let channel_key = deposit_channel_key(network, channel);
        assert_eq!(channel_key, format!("upto:deposit:{network}:{channel}"));
        assert_ne!(deposit_pending_key(network, &a), channel_key);
        assert!(deposit_pending_key(network, &a).starts_with("upto:deposit:"));
    }
}
