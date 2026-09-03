//! On-chain settlement for a verified Solana exact payment.

use std::str::FromStr;

use base64::Engine;
use compact_str::CompactString;
use r402_facilitator::{Duplicate, PendingSettlementStore};
use r402_protocol::payment::{Extensions, SettleRequest, SettleResponse};
use r402_protocol::{Base64Bytes, ChainProvider, ErrorReason, FacilitatorError, VerificationError};
use sha2::{Digest, Sha256};
use solana_client::rpc_response::{TransactionError, UiTransactionError};
use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
#[cfg(feature = "telemetry")]
use tracing_core::Level;

use super::verify::{VerifyTransferResult, verify_transfer};
use super::{SolanaExactFacilitator, enforce_memo};
use crate::chain::provider::{SolanaChainProviderError, SolanaChainProviderLike};
use crate::exact::payload::{self, TransactionInt};

/// Base64 SHA-256 of versioned **message** bytes (not the signed wire tx).
///
/// Slot 0 is the fee-payer signature the facilitator overwrites, so a wire-bytes
/// cache key is attacker-controlled. Official `transactionMessageHash`.
#[must_use]
pub fn transaction_message_hash(tx: &VersionedTransaction) -> String {
    let bytes = tx.message.serialize();
    base64::engine::general_purpose::STANDARD.encode(Sha256::digest(bytes))
}

fn decode_payload_transaction(
    transaction_b64: &str,
) -> Result<VersionedTransaction, FacilitatorError> {
    let raw = Base64Bytes::from(transaction_b64.as_bytes())
        .decode()
        .map_err(|e| VerificationError::InvalidFormat(e.to_string()))?;
    bincode::deserialize(&raw).map_err(|e| VerificationError::InvalidFormat(e.to_string()).into())
}

/// Signs and submits a verified transaction.
///
/// # Errors
///
/// Returns [`SolanaChainProviderError`] if signing or confirmation fails.
pub async fn settle_transaction<P: SolanaChainProviderLike>(
    provider: &P,
    verification: VerifyTransferResult,
) -> Result<Signature, SolanaChainProviderError> {
    match broadcast_payment(provider, verification).await {
        Ok(signature) => Ok(signature),
        Err(failure) => Err(*failure.error),
    }
}

struct BroadcastFailure {
    signature: Option<Signature>,
    error: Box<SolanaChainProviderError>,
}

async fn broadcast_payment<P: SolanaChainProviderLike>(
    provider: &P,
    verification: VerifyTransferResult,
) -> Result<Signature, BroadcastFailure> {
    let tx = TransactionInt::new(verification.transaction)
        .sign(provider)
        .map_err(|error| BroadcastFailure {
            signature: None,
            error: Box::new(error),
        })?;
    if !tx.is_fully_signed() {
        #[cfg(feature = "telemetry")]
        tracing::event!(Level::WARN, status = "failed", "undersigned transaction");
        return Err(BroadcastFailure {
            signature: None,
            error: Box::new(SolanaChainProviderError::InvalidTransaction(
                UiTransactionError::from(TransactionError::SignatureFailure),
            )),
        });
    }
    match provider.send(tx.inner()).await {
        Ok(signature) => match provider
            .send_and_confirm(tx.inner(), CommitmentConfig::confirmed())
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

fn should_release(outcome: &Result<SettleResponse, FacilitatorError>) -> bool {
    match outcome {
        Ok(SettleResponse::Success { .. }) => false,
        Ok(SettleResponse::Failure { transaction, .. }) => transaction.is_empty(),
        Ok(_) | Err(_) => true,
    }
}

fn settle_success(
    payer: &str,
    signature: Signature,
    network: &str,
    amount: &str,
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

fn pending_failure(signature: &str, payer: &str, network: &str, message: &str) -> SettleResponse {
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

async fn reconcile_pending<P: SolanaChainProviderLike>(
    provider: &P,
    pending: &dyn PendingSettlementStore,
    key: &str,
    cached: CompactString,
    payer: &str,
    network: &str,
) -> Result<SettleResponse, FacilitatorError> {
    let parsed = Signature::from_str(cached.as_str()).map_err(|e| {
        FacilitatorError::Onchain(format!("invalid pending settlement signature: {e}"))
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
        Ok(status) if status.value => Ok(SettleResponse::Success {
            payer: Some(payer.into()),
            transaction: parsed.to_string().into(),
            network: network.into(),
            amount: None,
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        }),
        Ok(_) | Err(_) => {
            pending.set(key, cached.clone());
            Ok(pending_failure(
                cached.as_str(),
                payer,
                network,
                "settlement still pending",
            ))
        }
    }
}

fn token_payer(tx: &VersionedTransaction) -> String {
    let Some(ix) = tx.message.instructions().get(2) else {
        return String::new();
    };
    let keys = tx.message.static_account_keys();
    ix.accounts
        .get(3)
        .and_then(|idx| keys.get(usize::from(*idx)))
        .map(ToString::to_string)
        .unwrap_or_default()
}

/// Deduplicates, re-verifies, and submits the payment.
pub(super) async fn settle<P>(
    facilitator: &SolanaExactFacilitator<P>,
    request: SettleRequest,
) -> Result<SettleResponse, FacilitatorError>
where
    P: SolanaChainProviderLike + ChainProvider + Send + Sync,
{
    let request = payload::v2::SettleRequest::from_settle(request)?;
    let decoded = decode_payload_transaction(&request.payment_payload.payload.transaction)?;
    let tx_key = transaction_message_hash(&decoded);
    let network = facilitator.provider.chain_id().to_string();

    if let Some(cached) = facilitator.pending.get(&tx_key) {
        facilitator.pending.delete(&tx_key);
        return reconcile_pending(
            &facilitator.provider,
            facilitator.pending.as_ref(),
            &tx_key,
            cached,
            &token_payer(&decoded),
            &network,
        )
        .await;
    }

    if facilitator.settlement_cache.reserve(tx_key.clone()) == Duplicate::Yes {
        return Err(VerificationError::DuplicateSettlement.into());
    }

    let outcome = settle_reserved(facilitator, &request, &tx_key, &network).await;
    if should_release(&outcome) {
        facilitator.settlement_cache.release(&tx_key);
    }
    outcome
}

async fn settle_reserved<P>(
    facilitator: &SolanaExactFacilitator<P>,
    request: &payload::v2::SettleRequest,
    tx_key: &str,
    network: &str,
) -> Result<SettleResponse, FacilitatorError>
where
    P: SolanaChainProviderLike + ChainProvider + Send + Sync,
{
    enforce_memo(request)?;
    let verification = verify_transfer(&facilitator.provider, request, &facilitator.config).await?;
    let amount = verification.amount.to_string();
    let payer = verification.payer.to_string();
    match broadcast_payment(&facilitator.provider, verification).await {
        Ok(signature) => {
            facilitator.pending.delete(tx_key);
            Ok(settle_success(&payer, signature, network, &amount))
        }
        Err(BroadcastFailure {
            signature: None,
            error,
        }) => Err((*error).into()),
        Err(BroadcastFailure {
            signature: Some(signature),
            error,
        }) => {
            if matches!(
                error.as_ref(),
                SolanaChainProviderError::InvalidTransaction(_)
            ) {
                Err((*error).into())
            } else {
                facilitator
                    .pending
                    .set(tx_key, signature.to_string().into());
                Ok(pending_failure(
                    &signature.to_string(),
                    &payer,
                    network,
                    &error.to_string(),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use solana_message::{Message, VersionedMessage};
    use solana_pubkey::Pubkey;
    use solana_transaction::Instruction;

    use super::*;

    fn dummy_tx(memo: &[u8]) -> VersionedTransaction {
        let payer = Pubkey::new_from_array([1u8; 32]);
        let ixs = [
            Instruction {
                program_id: Pubkey::new_from_array([2u8; 32]),
                accounts: Vec::new(),
                data: vec![12],
            },
            Instruction {
                program_id: crate::exact::SPL_MEMO_PROGRAM,
                accounts: Vec::new(),
                data: memo.to_vec(),
            },
        ];
        VersionedTransaction {
            signatures: vec![Signature::from([3u8; 64])],
            message: VersionedMessage::Legacy(Message::new(&ixs, Some(&payer))),
        }
    }

    #[test]
    fn slot_zero_signature_does_not_change_message_hash() {
        let a = dummy_tx(b"nonce-a");
        let mut b = a.clone();
        b.signatures[0] = Signature::from([9u8; 64]);
        assert_eq!(transaction_message_hash(&a), transaction_message_hash(&b));
        let c = dummy_tx(b"nonce-b");
        assert_ne!(transaction_message_hash(&a), transaction_message_hash(&c));
    }
}
