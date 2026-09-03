//! On-chain settlement for a verified Solana exact payment.

use std::str::FromStr;

use base64::Engine;
use compact_str::CompactString;
use r402_facilitator::{Duplicate, PendingSettlementStore};
use r402_protocol::payment::{Extensions, SettleRequest, SettleResponse};
use r402_protocol::{Base64Bytes, ChainProvider, ErrorReason, FacilitatorError, VerificationError};
use sha2::{Digest, Sha256};
use solana_client::client_error::ClientErrorKind;
use solana_client::rpc_client::SerializableTransaction;
use solana_client::rpc_response::{TransactionError, UiTransactionError};
use solana_commitment_config::CommitmentConfig;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
#[cfg(feature = "telemetry")]
use tracing_core::Level;

use super::smart_wallet::{has_static_transfer_layout, verify_post_settlement};
use super::verify::{TransferRequirement, VerifyTransferResult, verify_transfer};
use super::{SolanaExactFacilitator, enforce_memo};
use crate::chain::provider::{SignatureConfirm, SolanaChainProviderError, SolanaChainProviderLike};
use crate::exact::payload::{self, TransactionInt};

/// Official SVM settle failure when a broadcast tx is rejected on-chain.
const TRANSACTION_FAILED: &str = "invalid_exact_svm_transaction_failed";
/// Official Path 2 post-settle TOCTOU failure (spec §3.4).
const POST_SETTLEMENT_TRANSFER_NOT_CONFIRMED: &str = "post_settlement_transfer_not_confirmed";

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
    match broadcast_payment(provider, verification.transaction).await {
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
    transaction: VersionedTransaction,
) -> Result<Signature, BroadcastFailure> {
    let tx = TransactionInt::new(transaction)
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
    let signature = *tx.inner().get_signature();
    if let Err(error) = provider.send(tx.inner()).await {
        return Err(BroadcastFailure {
            signature: None,
            error: Box::new(error),
        });
    }
    match provider
        .confirm_signature(&signature, CommitmentConfig::confirmed())
        .await
    {
        Ok(SignatureConfirm::Confirmed) => Ok(signature),
        Ok(SignatureConfirm::OnchainFailure(err)) => Err(BroadcastFailure {
            signature: Some(signature),
            error: Box::new(SolanaChainProviderError::InvalidTransaction(err)),
        }),
        Ok(SignatureConfirm::TimedOut) => Err(BroadcastFailure {
            signature: Some(signature),
            error: Box::new(SolanaChainProviderError::Transport(Box::new(
                ClientErrorKind::Custom("Transaction confirmation timeout".to_owned()),
            ))),
        }),
        Err(error) => Err(BroadcastFailure {
            signature: Some(signature),
            error: Box::new(error),
        }),
    }
}

fn should_release(outcome: &Result<SettleResponse, FacilitatorError>) -> bool {
    match outcome {
        Ok(SettleResponse::Success { .. }) => false,
        Ok(SettleResponse::Failure { reason, .. }) => {
            *reason != ErrorReason::SettlementPending
                && reason.as_str() != POST_SETTLEMENT_TRANSFER_NOT_CONFIRMED
        }
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

fn transaction_failed(
    signature: &str,
    payer: &str,
    network: &str,
    message: &str,
) -> SettleResponse {
    SettleResponse::Failure {
        reason: ErrorReason::from_wire(TRANSACTION_FAILED),
        message: Some(message.into()),
        payer: Some(payer.into()),
        transaction: signature.into(),
        network: network.into(),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    }
}

fn post_settlement_failed(signature: &str, payer: &str, network: &str) -> SettleResponse {
    SettleResponse::Failure {
        reason: ErrorReason::from_wire(POST_SETTLEMENT_TRANSFER_NOT_CONFIRMED),
        message: None,
        payer: Some(payer.into()),
        transaction: signature.into(),
        network: network.into(),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "reconcile needs pending, path, and requirement together"
)]
async fn reconcile_pending<P: SolanaChainProviderLike>(
    provider: &P,
    pending: &dyn PendingSettlementStore,
    key: &str,
    cached: CompactString,
    payer: &str,
    network: &str,
    smart_wallet: bool,
    requirement: &TransferRequirement<'_>,
) -> Result<SettleResponse, FacilitatorError> {
    let parsed = Signature::from_str(cached.as_str()).map_err(|e| {
        FacilitatorError::Onchain(format!("invalid pending settlement signature: {e}"))
    })?;
    match provider
        .confirm_signature(&parsed, CommitmentConfig::confirmed())
        .await
    {
        Ok(SignatureConfirm::Confirmed) => {
            if smart_wallet {
                let check = verify_post_settlement(
                    provider,
                    &parsed,
                    requirement,
                    &[provider.pubkey()],
                    None,
                    None,
                )
                .await;
                if !check.verified {
                    return Ok(post_settlement_failed(cached.as_str(), payer, network));
                }
            }
            Ok(SettleResponse::Success {
                payer: Some(payer.into()),
                transaction: parsed.to_string().into(),
                network: network.into(),
                amount: None,
                extensions: Extensions::new(),
                extension_responses: Extensions::new(),
                extra: None,
            })
        }
        Ok(SignatureConfirm::OnchainFailure(err)) => {
            pending.delete(key);
            Ok(transaction_failed(
                cached.as_str(),
                payer,
                network,
                &err.to_string(),
            ))
        }
        Ok(SignatureConfirm::TimedOut) | Err(_) => {
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
    // Official `isDuplicate` (check+insert) is sync before any await; pending
    // get follows so a concurrent retry after `pending.delete` hits the lock.
    let duplicate = facilitator.settlement_cache.reserve(tx_key.clone()) == Duplicate::Yes;

    if let Some(cached) = facilitator.pending.get(&tx_key) {
        facilitator.pending.delete(&tx_key);
        let smart_wallet = facilitator.config.enable_smart_wallet_verification
            && !has_static_transfer_layout(&decoded);
        let requirement = TransferRequirement {
            pay_to: &request.payment_requirements.pay_to,
            asset: &request.payment_requirements.asset,
            amount: request.payment_requirements.amount.inner(),
        };
        let outcome = reconcile_pending(
            &facilitator.provider,
            facilitator.pending.as_ref(),
            &tx_key,
            cached,
            &token_payer(&decoded),
            &network,
            smart_wallet,
            &requirement,
        )
        .await;
        if should_release(&outcome) {
            facilitator.settlement_cache.release(&tx_key);
        }
        return outcome;
    }

    if duplicate {
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
    let requirement = TransferRequirement {
        pay_to: &request.payment_requirements.pay_to,
        asset: &request.payment_requirements.asset,
        amount: request.payment_requirements.amount.inner(),
    };
    settle_verified(facilitator, verification, tx_key, network, &requirement).await
}

async fn settle_verified<P>(
    facilitator: &SolanaExactFacilitator<P>,
    verification: VerifyTransferResult,
    tx_key: &str,
    network: &str,
    requirement: &TransferRequirement<'_>,
) -> Result<SettleResponse, FacilitatorError>
where
    P: SolanaChainProviderLike + ChainProvider + Send + Sync,
{
    let amount = verification.amount.to_string();
    let payer = verification.payer.to_string();
    let matched = verification.matched_transfer;
    let known_destination = matched.map(|t| t.destination);
    let balance_before = if let Some(dest) = known_destination {
        facilitator
            .provider
            .get_token_account_balance(&dest)
            .await
            .ok()
            .flatten()
    } else {
        None
    };
    match broadcast_payment(&facilitator.provider, verification.transaction).await {
        Ok(signature) => {
            facilitator.pending.delete(tx_key);
            if matched.is_some() {
                let check = verify_post_settlement(
                    &facilitator.provider,
                    &signature,
                    requirement,
                    &[facilitator.provider.pubkey()],
                    balance_before,
                    known_destination,
                )
                .await;
                if !check.verified {
                    return Ok(post_settlement_failed(
                        &signature.to_string(),
                        &payer,
                        network,
                    ));
                }
            }
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
                facilitator.pending.delete(tx_key);
                Ok(transaction_failed(
                    &signature.to_string(),
                    &payer,
                    network,
                    &error.to_string(),
                ))
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
    use std::future::Future;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use r402_facilitator::{Facilitator, InMemoryPendingSettlementStore, SettlementCache};
    use r402_protocol::payment::SettleRequest;
    use r402_protocol::{ChainId, ChainProvider};
    use solana_account::Account;
    use solana_client::nonblocking::rpc_client::RpcClient;
    use solana_client::rpc_config::RpcSimulateTransactionConfig;
    use solana_commitment_config::CommitmentConfig;
    use solana_message::{Message, VersionedMessage};
    use solana_pubkey::Pubkey;
    use solana_transaction::Instruction;

    use super::*;
    use crate::chain::account::{Address, SolanaChainReference};
    use crate::chain::provider::SignatureConfirm;
    use crate::exact::SPL_MEMO_PROGRAM;
    use crate::exact::facilitator::{SolanaExactFacilitator, SolanaExactFacilitatorConfig};

    fn dummy_tx(memo: &[u8]) -> VersionedTransaction {
        let payer = Pubkey::new_from_array([1u8; 32]);
        let ixs = [
            Instruction {
                program_id: Pubkey::new_from_array([2u8; 32]),
                accounts: Vec::new(),
                data: vec![12],
            },
            Instruction {
                program_id: SPL_MEMO_PROGRAM,
                accounts: Vec::new(),
                data: memo.to_vec(),
            },
        ];
        VersionedTransaction {
            signatures: vec![Signature::from([3u8; 64])],
            message: VersionedMessage::Legacy(Message::new(&ixs, Some(&payer))),
        }
    }

    fn wire_settle(tx: &VersionedTransaction) -> SettleRequest {
        let b64 = TransactionInt::new(tx.clone()).as_base64().expect("b64");
        let reqs = serde_json::json!({
            "scheme": "exact",
            "network": "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
            "amount": "1000000",
            "payTo": "11111111111111111111111111111111",
            "maxTimeoutSeconds": 60,
            "asset": "11111111111111111111111111111111",
            "extra": { "feePayer": "11111111111111111111111111111111" }
        });
        serde_json::json!({
            "x402Version": 2,
            "paymentPayload": {
                "x402Version": 2,
                "accepted": reqs,
                "payload": { "transaction": b64 }
            },
            "paymentRequirements": reqs
        })
        .into()
    }

    struct StubProvider {
        pubkey: Pubkey,
        send_count: AtomicUsize,
        send_ok: bool,
        confirm: Mutex<SignatureConfirm>,
        broadcast_sig: Signature,
        inner_ixs: Mutex<Option<Vec<solana_client::rpc_response::UiInnerInstructions>>>,
        token_balances: Mutex<std::collections::HashMap<Pubkey, u64>>,
        balance_reads: AtomicUsize,
        balance_after: Mutex<Option<u64>>,
    }

    impl StubProvider {
        fn new(confirm: SignatureConfirm) -> Self {
            Self {
                pubkey: Pubkey::new_from_array([9u8; 32]),
                send_count: AtomicUsize::new(0),
                send_ok: true,
                confirm: Mutex::new(confirm),
                broadcast_sig: Signature::from([7u8; 64]),
                inner_ixs: Mutex::new(None),
                token_balances: Mutex::new(std::collections::HashMap::new()),
                balance_reads: AtomicUsize::new(0),
                balance_after: Mutex::new(None),
            }
        }

        fn send_result(&self) -> Result<Signature, SolanaChainProviderError> {
            if self.send_ok {
                Ok(self.broadcast_sig)
            } else {
                Err(SolanaChainProviderError::Custom("send failed".into()))
            }
        }
    }

    impl ChainProvider for StubProvider {
        fn signer_addresses(&self) -> Vec<String> {
            vec![self.pubkey.to_string()]
        }

        fn chain_id(&self) -> ChainId {
            SolanaChainReference::SOLANA_DEVNET.into()
        }
    }

    impl SolanaChainProviderLike for StubProvider {
        fn simulate_transaction_with_config(
            &self,
            _tx: &VersionedTransaction,
            _cfg: RpcSimulateTransactionConfig,
        ) -> impl Future<
            Output = Result<
                crate::chain::provider::SimulateTransactionResult,
                SolanaChainProviderError,
            >,
        > + Send {
            std::future::ready(Err(SolanaChainProviderError::Custom("no simulate".into())))
        }

        fn get_multiple_accounts(
            &self,
            _pubkeys: &[Pubkey],
        ) -> impl Future<Output = Result<Vec<Option<Account>>, SolanaChainProviderError>> + Send
        {
            std::future::ready(Ok(Vec::new()))
        }

        fn max_compute_unit_limit(&self) -> u32 {
            200_000
        }

        fn max_compute_unit_price(&self) -> u64 {
            1
        }

        fn pubkey(&self) -> Pubkey {
            self.pubkey
        }

        fn fee_payer(&self) -> Address {
            Address::new(self.pubkey)
        }

        fn sign(
            &self,
            mut tx: VersionedTransaction,
        ) -> Result<VersionedTransaction, SolanaChainProviderError> {
            if tx.signatures.is_empty() {
                tx.signatures.push(self.broadcast_sig);
            } else {
                tx.signatures[0] = self.broadcast_sig;
            }
            Ok(tx)
        }

        fn send_and_confirm(
            &self,
            _tx: &VersionedTransaction,
            _commitment_config: CommitmentConfig,
        ) -> impl Future<Output = Result<Signature, SolanaChainProviderError>> + Send {
            std::future::ready(Err(SolanaChainProviderError::Custom("unused".into())))
        }

        fn rpc_client(&self) -> Arc<RpcClient> {
            Arc::new(RpcClient::new("http://127.0.0.1:9".to_owned()))
        }

        fn send(
            &self,
            _tx: &VersionedTransaction,
        ) -> impl Future<Output = Result<Signature, SolanaChainProviderError>> + Send {
            self.send_count.fetch_add(1, Ordering::SeqCst);
            std::future::ready(self.send_result())
        }

        fn confirm_signature(
            &self,
            _signature: &Signature,
            _commitment_config: CommitmentConfig,
        ) -> impl Future<Output = Result<SignatureConfirm, SolanaChainProviderError>> + Send
        {
            let confirm = self
                .confirm
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            std::future::ready(Ok(confirm))
        }

        fn get_confirmed_inner_instructions(
            &self,
            _signature: &Signature,
        ) -> impl Future<
            Output = Result<
                Option<Vec<solana_client::rpc_response::UiInnerInstructions>>,
                SolanaChainProviderError,
            >,
        > + Send {
            let inner = self
                .inner_ixs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            std::future::ready(Ok(inner))
        }

        fn get_token_account_balance(
            &self,
            token_account: &Pubkey,
        ) -> impl Future<Output = Result<Option<u64>, SolanaChainProviderError>> + Send {
            let n = self.balance_reads.fetch_add(1, Ordering::SeqCst);
            let stored = self
                .token_balances
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(token_account)
                .copied();
            let after = *self
                .balance_after
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let bal = if n >= 1 { after.or(stored) } else { stored };
            std::future::ready(Ok(bal))
        }
    }

    fn facilitator(
        stub: StubProvider,
        pending: Arc<InMemoryPendingSettlementStore>,
        cache: SettlementCache,
    ) -> SolanaExactFacilitator<StubProvider> {
        SolanaExactFacilitator::with_settlement_cache(
            stub,
            SolanaExactFacilitatorConfig::default(),
            cache,
        )
        .with_pending_store(pending)
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

    #[test]
    fn should_release_keeps_lock_only_for_settlement_pending() {
        let pending = pending_failure("sig", "payer", "solana:devnet", "wait");
        assert!(!should_release(&Ok(pending)));
        let failed = transaction_failed("sig", "payer", "solana:devnet", "err");
        assert!(should_release(&Ok(failed)));
        assert!(should_release(&Err(
            VerificationError::DuplicateSettlement.into()
        )));
    }

    #[tokio::test]
    async fn confirm_timeout_records_pending_by_message_hash() {
        let tx = dummy_tx(b"timeout");
        let tx_key = transaction_message_hash(&tx);
        let pending = Arc::new(InMemoryPendingSettlementStore::new());
        let stub = StubProvider::new(SignatureConfirm::TimedOut);
        let sig = stub.broadcast_sig;
        let fac = facilitator(stub, Arc::clone(&pending), SettlementCache::new());
        let pay_to = Address::new(Pubkey::new_from_array([1u8; 32]));
        let asset = Address::new(Pubkey::new_from_array([2u8; 32]));
        let requirement = TransferRequirement {
            asset: &asset,
            pay_to: &pay_to,
            amount: 1,
        };
        let verification = VerifyTransferResult {
            payer: Address::new(Pubkey::new_from_array([4u8; 32])),
            transaction: tx,
            amount: 1,
            matched_transfer: None,
        };
        let response = settle_verified(&fac, verification, &tx_key, "solana:devnet", &requirement)
            .await
            .expect("settle");
        assert!(response.is_retryable_settlement_pending());
        let expected = sig.to_string();
        assert_eq!(pending.get(&tx_key).as_deref(), Some(expected.as_str()));
    }

    #[tokio::test]
    async fn onchain_failure_after_send_is_terminal_with_signature() {
        let tx = dummy_tx(b"onchain");
        let tx_key = transaction_message_hash(&tx);
        let pending = Arc::new(InMemoryPendingSettlementStore::new());
        let stub = StubProvider::new(SignatureConfirm::OnchainFailure(UiTransactionError::from(
            TransactionError::InsufficientFundsForFee,
        )));
        let sig = stub.broadcast_sig;
        let cache = SettlementCache::new();
        assert_eq!(cache.reserve(tx_key.clone()), Duplicate::No);
        let fac = facilitator(stub, Arc::clone(&pending), cache.clone());
        let pay_to = Address::new(Pubkey::new_from_array([1u8; 32]));
        let asset = Address::new(Pubkey::new_from_array([2u8; 32]));
        let requirement = TransferRequirement {
            asset: &asset,
            pay_to: &pay_to,
            amount: 1,
        };
        let verification = VerifyTransferResult {
            payer: Address::new(Pubkey::new_from_array([4u8; 32])),
            transaction: tx,
            amount: 1,
            matched_transfer: None,
        };
        let response = settle_verified(&fac, verification, &tx_key, "solana:devnet", &requirement)
            .await
            .expect("settle");
        match response {
            SettleResponse::Failure {
                reason,
                transaction,
                ..
            } => {
                assert_eq!(reason.as_str(), TRANSACTION_FAILED);
                assert_eq!(transaction.as_str(), sig.to_string());
            }
            other => panic!("expected failure, got {other:?}"),
        }
        assert!(pending.get(&tx_key).is_none());
        cache.release(&tx_key);
        assert_eq!(cache.reserve(tx_key), Duplicate::No);
    }

    #[tokio::test]
    async fn pending_hit_skips_send_and_reconciles() {
        let tx = dummy_tx(b"hit");
        let tx_key = transaction_message_hash(&tx);
        let cached = Signature::from([8u8; 64]);
        let pending = Arc::new(InMemoryPendingSettlementStore::new());
        pending.set(&tx_key, cached.to_string().into());
        let stub = StubProvider::new(SignatureConfirm::Confirmed);
        let fac = facilitator(stub, Arc::clone(&pending), SettlementCache::new());
        let response = Facilitator::settle(&fac, wire_settle(&tx))
            .await
            .expect("settle");
        match response {
            SettleResponse::Success { transaction, .. } => {
                assert_eq!(transaction.as_str(), cached.to_string());
            }
            other => panic!("expected success, got {other:?}"),
        }
        assert_eq!(fac.provider.send_count.load(Ordering::SeqCst), 0);
        assert!(pending.get(&tx_key).is_none());
    }

    #[tokio::test]
    async fn pending_hit_still_unconfirmed_restores_store() {
        let tx = dummy_tx(b"still");
        let tx_key = transaction_message_hash(&tx);
        let cached = Signature::from([8u8; 64]);
        let pending = Arc::new(InMemoryPendingSettlementStore::new());
        pending.set(&tx_key, cached.to_string().into());
        let stub = StubProvider::new(SignatureConfirm::TimedOut);
        let fac = facilitator(stub, Arc::clone(&pending), SettlementCache::new());
        let response = Facilitator::settle(&fac, wire_settle(&tx))
            .await
            .expect("settle");
        assert!(response.is_retryable_settlement_pending());
        let expected = cached.to_string();
        assert_eq!(pending.get(&tx_key).as_deref(), Some(expected.as_str()));
        assert_eq!(fac.provider.send_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn verify_failure_after_reserve_releases_lock() {
        let tx = dummy_tx(b"verify-fail");
        let tx_key = transaction_message_hash(&tx);
        let pending = Arc::new(InMemoryPendingSettlementStore::new());
        let cache = SettlementCache::new();
        let stub = StubProvider::new(SignatureConfirm::Confirmed);
        let fac = facilitator(stub, pending, cache.clone());
        let first = Facilitator::settle(&fac, wire_settle(&tx)).await;
        assert!(first.is_err());
        assert_eq!(cache.reserve(tx_key), Duplicate::No);
    }

    #[tokio::test]
    async fn send_failure_never_writes_pending() {
        let tx = dummy_tx(b"send-fail");
        let tx_key = transaction_message_hash(&tx);
        let pending = Arc::new(InMemoryPendingSettlementStore::new());
        let mut stub = StubProvider::new(SignatureConfirm::Confirmed);
        stub.send_ok = false;
        let fac = facilitator(stub, Arc::clone(&pending), SettlementCache::new());
        let pay_to = Address::new(Pubkey::new_from_array([1u8; 32]));
        let asset = Address::new(Pubkey::new_from_array([2u8; 32]));
        let requirement = TransferRequirement {
            asset: &asset,
            pay_to: &pay_to,
            amount: 1,
        };
        let verification = VerifyTransferResult {
            payer: Address::new(Pubkey::new_from_array([4u8; 32])),
            transaction: tx,
            amount: 1,
            matched_transfer: None,
        };
        let result =
            settle_verified(&fac, verification, &tx_key, "solana:devnet", &requirement).await;
        assert!(result.is_err());
        assert!(pending.get(&tx_key).is_none());
    }

    #[test]
    fn should_release_keeps_lock_on_post_settlement_failure() {
        let failed = post_settlement_failed("sig", "payer", "solana:devnet");
        assert!(!should_release(&Ok(failed)));
    }

    #[tokio::test]
    async fn path2_post_settle_inner_miss_is_failure() {
        use solana_client::rpc_response::{
            ParsedInstruction, UiInnerInstructions, UiInstruction, UiParsedInstruction,
        };

        use crate::exact::ATA_PROGRAM_PUBKEY;
        use crate::exact::facilitator::smart_wallet::ObservedTransfer;

        let pay_to = Address::new(Pubkey::new_from_array([1u8; 32]));
        let asset = Address::new(Pubkey::new_from_array([2u8; 32]));
        let dest = {
            let (ata, _) = Pubkey::find_program_address(
                &[pay_to.as_ref(), spl_token::ID.as_ref(), asset.as_ref()],
                &ATA_PROGRAM_PUBKEY,
            );
            ata
        };
        let matched = ObservedTransfer {
            program_id: spl_token::ID,
            amount: 1,
            mint: *asset.pubkey(),
            destination: dest,
            authority: Pubkey::new_from_array([4u8; 32]),
        };
        let miss = vec![UiInnerInstructions {
            index: 0,
            instructions: vec![UiInstruction::Parsed(UiParsedInstruction::Parsed(
                ParsedInstruction {
                    program: "spl-token".into(),
                    program_id: spl_token::ID.to_string(),
                    parsed: serde_json::json!({
                        "type": "transferChecked",
                        "info": {
                            "mint": matched.mint.to_string(),
                            "destination": Pubkey::new_from_array([8u8; 32]).to_string(),
                            "authority": matched.authority.to_string(),
                            "tokenAmount": { "amount": "1" }
                        }
                    }),
                    stack_height: None,
                },
            ))],
        }];

        let tx = dummy_tx(b"toctou");
        let tx_key = transaction_message_hash(&tx);
        let pending = Arc::new(InMemoryPendingSettlementStore::new());
        let stub = StubProvider::new(SignatureConfirm::Confirmed);
        stub.inner_ixs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(miss);
        let fac = facilitator(stub, Arc::clone(&pending), SettlementCache::new());
        let requirement = TransferRequirement {
            asset: &asset,
            pay_to: &pay_to,
            amount: 1,
        };
        let verification = VerifyTransferResult {
            payer: Address::new(matched.authority),
            transaction: tx,
            amount: 1,
            matched_transfer: Some(matched),
        };
        let response = settle_verified(&fac, verification, &tx_key, "solana:devnet", &requirement)
            .await
            .expect("settle");
        match response {
            SettleResponse::Failure { reason, .. } => {
                assert_eq!(reason.as_str(), POST_SETTLEMENT_TRANSFER_NOT_CONFIRMED);
            }
            other => panic!("expected post-settle failure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn path2_post_settle_unverified_is_failure() {
        use crate::exact::ATA_PROGRAM_PUBKEY;
        use crate::exact::facilitator::smart_wallet::ObservedTransfer;

        let pay_to = Address::new(Pubkey::new_from_array([1u8; 32]));
        let asset = Address::new(Pubkey::new_from_array([2u8; 32]));
        let dest = {
            let (ata, _) = Pubkey::find_program_address(
                &[pay_to.as_ref(), spl_token::ID.as_ref(), asset.as_ref()],
                &ATA_PROGRAM_PUBKEY,
            );
            ata
        };
        let matched = ObservedTransfer {
            program_id: spl_token::ID,
            amount: 1,
            mint: *asset.pubkey(),
            destination: dest,
            authority: Pubkey::new_from_array([4u8; 32]),
        };
        let tx = dummy_tx(b"unverified");
        let tx_key = transaction_message_hash(&tx);
        let pending = Arc::new(InMemoryPendingSettlementStore::new());
        let stub = StubProvider::new(SignatureConfirm::Confirmed);
        let fac = facilitator(stub, pending, SettlementCache::new());
        let requirement = TransferRequirement {
            asset: &asset,
            pay_to: &pay_to,
            amount: 1,
        };
        let verification = VerifyTransferResult {
            payer: Address::new(matched.authority),
            transaction: tx,
            amount: 1,
            matched_transfer: Some(matched),
        };
        let response = settle_verified(&fac, verification, &tx_key, "solana:devnet", &requirement)
            .await
            .expect("settle");
        match response {
            SettleResponse::Failure { reason, .. } => {
                assert_eq!(reason.as_str(), POST_SETTLEMENT_TRANSFER_NOT_CONFIRMED);
            }
            other => panic!("expected unverified failure, got {other:?}"),
        }
    }

    fn path2_matched(
        dest: Pubkey,
        asset: &Address,
    ) -> crate::exact::facilitator::smart_wallet::ObservedTransfer {
        crate::exact::facilitator::smart_wallet::ObservedTransfer {
            program_id: spl_token::ID,
            amount: 10,
            mint: *asset.pubkey(),
            destination: dest,
            authority: Pubkey::new_from_array([4u8; 32]),
        }
    }

    fn path2_dest_ata(pay_to: &Address, asset: &Address) -> Pubkey {
        let (ata, _) = Pubkey::find_program_address(
            &[pay_to.as_ref(), spl_token::ID.as_ref(), asset.as_ref()],
            &crate::exact::ATA_PROGRAM_PUBKEY,
        );
        ata
    }

    #[tokio::test]
    async fn path2_post_settle_balance_delta_success() {
        let pay_to = Address::new(Pubkey::new_from_array([1u8; 32]));
        let asset = Address::new(Pubkey::new_from_array([2u8; 32]));
        let dest = path2_dest_ata(&pay_to, &asset);
        let matched = path2_matched(dest, &asset);
        let requirement = TransferRequirement {
            asset: &asset,
            pay_to: &pay_to,
            amount: 10,
        };
        let tx = dummy_tx(b"delta-ok");
        let tx_key = transaction_message_hash(&tx);
        let pending = Arc::new(InMemoryPendingSettlementStore::new());
        let stub = StubProvider::new(SignatureConfirm::Confirmed);
        stub.token_balances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(dest, 0);
        *stub
            .balance_after
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(10);
        let fac = facilitator(stub, pending, SettlementCache::new());
        let verification = VerifyTransferResult {
            payer: Address::new(matched.authority),
            transaction: tx,
            amount: 10,
            matched_transfer: Some(matched),
        };
        let response = settle_verified(&fac, verification, &tx_key, "solana:devnet", &requirement)
            .await
            .expect("settle");
        assert!(matches!(response, SettleResponse::Success { .. }));
    }

    #[tokio::test]
    async fn path2_post_settle_balance_delta_insufficient() {
        let pay_to = Address::new(Pubkey::new_from_array([1u8; 32]));
        let asset = Address::new(Pubkey::new_from_array([2u8; 32]));
        let dest = path2_dest_ata(&pay_to, &asset);
        let matched = path2_matched(dest, &asset);
        let requirement = TransferRequirement {
            asset: &asset,
            pay_to: &pay_to,
            amount: 10,
        };
        let tx = dummy_tx(b"delta-low");
        let tx_key = transaction_message_hash(&tx);
        let pending = Arc::new(InMemoryPendingSettlementStore::new());
        let stub = StubProvider::new(SignatureConfirm::Confirmed);
        stub.token_balances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(dest, 0);
        *stub
            .balance_after
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(1);
        let fac = facilitator(stub, pending, SettlementCache::new());
        let verification = VerifyTransferResult {
            payer: Address::new(matched.authority),
            transaction: tx,
            amount: 10,
            matched_transfer: Some(matched),
        };
        let response = settle_verified(&fac, verification, &tx_key, "solana:devnet", &requirement)
            .await
            .expect("settle");
        match response {
            SettleResponse::Failure { reason, .. } => {
                assert_eq!(reason.as_str(), POST_SETTLEMENT_TRANSFER_NOT_CONFIRMED);
            }
            other => panic!("expected insufficient delta failure, got {other:?}"),
        }
    }
}
