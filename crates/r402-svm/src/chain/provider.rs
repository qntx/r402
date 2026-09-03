use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;
use std::time::Duration;

use r402_protocol::{ChainId, ChainProvider, FacilitatorError, VerificationError};
use solana_account::Account;
use solana_client::client_error::{ClientError, ClientErrorKind};
use solana_client::nonblocking::pubsub_client::PubsubClient;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::pubsub_client::PubsubClientError;
use solana_client::rpc_client::SerializableTransaction;
use solana_client::rpc_config::{
    RpcSendTransactionConfig, RpcSignatureSubscribeConfig, RpcSimulateTransactionConfig,
    RpcTransactionConfig, UiTransactionEncoding,
};
use solana_client::rpc_response::{
    RpcSignatureResult, TransactionConfirmationStatus, TransactionError, UiInnerInstructions,
    UiLoadedAddresses, UiTransactionError,
};
use solana_commitment_config::CommitmentConfig;
use solana_keypair::{Keypair, Signer};
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_signer::SignerError;
use solana_transaction::versioned::VersionedTransaction;

use crate::chain::SOLANA_MAX_COMPUTE_UNIT_PRICE_MICROLAMPORTS;
use crate::chain::account::{Address, SolanaChainReference};

/// Errors that can occur when interacting with a Solana chain provider.
#[derive(thiserror::Error, Debug)]
pub enum SolanaChainProviderError {
    /// Failed to sign a transaction.
    #[error(transparent)]
    Signer(#[from] SignerError),
    /// The transaction was invalid or failed simulation.
    #[error("Invalid transaction: {0}")]
    InvalidTransaction(#[from] UiTransactionError),
    /// RPC transport error.
    #[error(transparent)]
    Transport(Box<ClientErrorKind>),
    /// `WebSocket` pubsub transport error.
    #[error(transparent)]
    PubsubTransport(#[from] PubsubClientError),
    /// A custom error message.
    #[error("{0}")]
    Custom(String),
}

impl From<ClientError> for SolanaChainProviderError {
    fn from(value: ClientError) -> Self {
        Self::Transport(value.kind)
    }
}

impl From<SolanaChainProviderError> for FacilitatorError {
    fn from(value: SolanaChainProviderError) -> Self {
        Self::Onchain(value.to_string())
    }
}

impl From<SolanaChainProviderError> for VerificationError {
    fn from(value: SolanaChainProviderError) -> Self {
        Self::SimulationFailed(value.to_string())
    }
}

/// Provider for interacting with a Solana blockchain.
///
/// This provider handles transaction signing, simulation, and submission for
/// Solana-based x402 payments. It supports both RPC polling and `WebSocket`
/// subscriptions for transaction confirmation.
///
/// # Configuration
///
/// The provider requires:
/// - A keypair for signing transactions (the fee payer)
/// - An RPC endpoint URL
/// - Optionally, a `WebSocket` pubsub URL for faster confirmations
/// - Compute unit limits and prices for transaction prioritization
pub struct SolanaChainProvider {
    /// The Solana network this provider connects to.
    chain: SolanaChainReference,
    /// The keypair used for signing transactions.
    keypair: Arc<Keypair>,
    /// The RPC client for sending requests.
    rpc_client: Arc<RpcClient>,
    /// Optional `WebSocket` client for subscriptions.
    pubsub_client: Option<Arc<PubsubClient>>,
    /// Maximum compute units allowed per transaction.
    max_compute_unit_limit: u32,
    /// Maximum price per compute unit (in micro-lamports).
    max_compute_unit_price: u64,
}

impl Debug for SolanaChainProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SolanaChainProvider")
            .field("pubkey", &self.keypair.pubkey())
            .field("chain", &self.chain)
            .field("rpc_url", &self.rpc_client.url())
            .field("max_compute_unit_limit", &self.max_compute_unit_limit)
            .field("max_compute_unit_price", &self.max_compute_unit_price)
            .finish_non_exhaustive()
    }
}

impl SolanaChainProvider {
    /// Creates a new Solana chain provider.
    ///
    /// # Parameters
    ///
    /// - `keypair`: The keypair used for signing transactions (fee payer)
    /// - `rpc_url`: The HTTP RPC endpoint URL
    /// - `pubsub_url`: Optional `WebSocket` pubsub endpoint for faster confirmations
    /// - `chain`: The Solana network identifier
    /// - `max_compute_unit_limit`: Maximum compute units per transaction
    /// - `max_compute_unit_price`: Maximum price per compute unit in micro-lamports.
    ///   `None` uses [`SOLANA_MAX_COMPUTE_UNIT_PRICE_MICROLAMPORTS`].
    ///
    /// # Errors
    ///
    /// Returns an error if the `WebSocket` connection fails to establish.
    pub async fn new(
        keypair: Keypair,
        rpc_url: String,
        pubsub_url: Option<String>,
        chain: SolanaChainReference,
        max_compute_unit_limit: u32,
        max_compute_unit_price: Option<u64>,
    ) -> Result<Self, PubsubClientError> {
        let max_compute_unit_price =
            max_compute_unit_price.unwrap_or(SOLANA_MAX_COMPUTE_UNIT_PRICE_MICROLAMPORTS);
        #[cfg(feature = "telemetry")]
        {
            let signer_addresses = vec![keypair.pubkey()];
            let chain_id: ChainId = chain.into();
            tracing::info!(
                chain = %chain_id,
                rpc = rpc_url,
                pubsub = ?pubsub_url,
                signers = ?signer_addresses,
                max_compute_unit_limit,
                max_compute_unit_price,
                "Using Solana provider"
            );
        }
        let rpc_client = RpcClient::new(rpc_url);
        let pubsub_client = if let Some(pubsub_url) = pubsub_url {
            let client = PubsubClient::new(pubsub_url).await?;
            Some(client)
        } else {
            None
        };
        Ok(Self {
            keypair: Arc::new(keypair),
            chain,
            rpc_client: Arc::new(rpc_client),
            pubsub_client: pubsub_client.map(Arc::new),
            max_compute_unit_limit,
            max_compute_unit_price,
        })
    }

    /// Returns a cloned reference to the RPC client.
    #[must_use]
    pub fn rpc_client(&self) -> Arc<RpcClient> {
        Arc::clone(&self.rpc_client)
    }

    /// Returns a cloned reference to the optional pubsub client.
    #[must_use]
    pub fn pubsub_client(&self) -> Option<Arc<PubsubClient>> {
        self.pubsub_client.clone()
    }

    /// Sends a signed transaction to the network without waiting for confirmation.
    ///
    /// This method submits the transaction with `skip_preflight: true` to avoid
    /// simulation delays. The transaction should already be signed.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC request fails or the transaction is rejected.
    pub async fn send(
        &self,
        tx: &VersionedTransaction,
    ) -> Result<Signature, SolanaChainProviderError> {
        let signature = self
            .rpc_client
            .send_transaction_with_config(
                tx,
                RpcSendTransactionConfig {
                    skip_preflight: true,
                    ..RpcSendTransactionConfig::default()
                },
            )
            .await?;
        Ok(signature)
    }
}

impl ChainProvider for SolanaChainProvider {
    fn signer_addresses(&self) -> Vec<String> {
        vec![self.fee_payer().to_string()]
    }

    fn chain_id(&self) -> ChainId {
        self.chain.into()
    }
}

/// Successful `simulateTransaction` outcome.
///
/// Simulation `err` is mapped to [`SolanaChainProviderError::InvalidTransaction`]
/// before this struct is returned, so callers only see CPI traces on a
/// simulation that would commit.
#[derive(Debug, Clone, Default)]
pub struct SimulateTransactionResult {
    /// CPI inner instructions (empty when the RPC omitted them).
    pub inner_instructions: Vec<UiInnerInstructions>,
    /// ALT-loaded addresses in runtime order (writable, then readonly).
    pub loaded_addresses: UiLoadedAddresses,
    /// Compute units consumed, when the RPC reports them.
    pub units_consumed: Option<u64>,
}

/// Result of waiting for a broadcast signature (official `getSignatureStatuses`).
#[derive(Debug, Clone)]
pub enum SignatureConfirm {
    /// Confirmed/finalized with no on-chain `err`.
    Confirmed,
    /// Confirmed/finalized with `entry.err` (official `TransactionOnchainFailureError`).
    OnchainFailure(UiTransactionError),
    /// Never observed as confirmed/finalized before the wait budget.
    TimedOut,
}

/// Trait for Solana chain provider operations.
///
/// This trait abstracts the core operations needed for x402 payment processing
/// on Solana, including transaction simulation, signing, and confirmation.
pub trait SolanaChainProviderLike: Sync {
    /// Simulates a transaction with the given configuration.
    ///
    /// Returns inner instructions when the RPC records them (`inner_instructions:
    /// true`). A simulation `err` is [`SolanaChainProviderError::InvalidTransaction`].
    fn simulate_transaction_with_config(
        &self,
        tx: &VersionedTransaction,
        cfg: RpcSimulateTransactionConfig,
    ) -> impl Future<Output = Result<SimulateTransactionResult, SolanaChainProviderError>> + Send;

    /// Fetches multiple accounts in a single RPC call.
    fn get_multiple_accounts(
        &self,
        pubkeys: &[Pubkey],
    ) -> impl Future<Output = Result<Vec<Option<Account>>, SolanaChainProviderError>> + Send;

    /// Returns the maximum compute unit limit for transactions.
    fn max_compute_unit_limit(&self) -> u32;

    /// Returns the maximum compute unit price in micro-lamports.
    fn max_compute_unit_price(&self) -> u64;

    /// Returns the public key of the fee payer.
    fn pubkey(&self) -> Pubkey;

    /// Returns the fee payer address.
    fn fee_payer(&self) -> Address;

    /// Signs a transaction with the provider's keypair.
    ///
    /// # Errors
    ///
    /// Returns [`SolanaChainProviderError`] if signing fails.
    fn sign(
        &self,
        tx: VersionedTransaction,
    ) -> Result<VersionedTransaction, SolanaChainProviderError>;

    /// Sends a transaction and waits for confirmation.
    ///
    /// Uses `WebSocket` subscription if available, otherwise polls for confirmation.
    fn send_and_confirm(
        &self,
        tx: &VersionedTransaction,
        commitment_config: CommitmentConfig,
    ) -> impl Future<Output = Result<Signature, SolanaChainProviderError>> + Send;

    /// Underlying RPC client for scheme-specific reads (channel accounts, slots).
    fn rpc_client(&self) -> Arc<RpcClient>;

    /// Broadcasts a signed transaction without waiting for confirmation.
    fn send(
        &self,
        tx: &VersionedTransaction,
    ) -> impl Future<Output = Result<Signature, SolanaChainProviderError>> + Send;

    /// Polls `getSignatureStatuses` until confirmed/finalized, on-chain `err`, or timeout.
    fn confirm_signature(
        &self,
        signature: &Signature,
        commitment_config: CommitmentConfig,
    ) -> impl Future<Output = Result<SignatureConfirm, SolanaChainProviderError>> + Send;

    /// Confirmed-tx inner instructions for Path 2 post-settle TOCTOU.
    ///
    /// `Ok(None)` when the tx is not yet indexed or the RPC omitted inners.
    /// `Ok(Some(vec))` (including empty) means the index returned a meta object.
    fn get_confirmed_inner_instructions(
        &self,
        _signature: &Signature,
    ) -> impl Future<Output = Result<Option<Vec<UiInnerInstructions>>, SolanaChainProviderError>> + Send
    {
        std::future::ready(Ok(None))
    }

    /// Token-account balance in base units, or `None` if missing/RPC error.
    fn get_token_account_balance(
        &self,
        _token_account: &Pubkey,
    ) -> impl Future<Output = Result<Option<u64>, SolanaChainProviderError>> + Send {
        std::future::ready(Ok(None))
    }

    /// Resolve Address Lookup Table addresses (spec §2.1.2).
    ///
    /// Default: not implemented. Path 2 requires a real implementation.
    fn fetch_address_lookup_tables(
        &self,
        _tables: &[Pubkey],
    ) -> impl Future<Output = Result<HashMap<Pubkey, Vec<Pubkey>>, SolanaChainProviderError>> + Send
    {
        std::future::ready(Err(SolanaChainProviderError::Custom(
            "fetchAddressLookupTables not implemented".into(),
        )))
    }

    /// Whether this provider implements Path 2 caps (simulate inners, confirmed
    /// inners, token balance, ALT fetch). Official constructor requires this.
    fn supports_smart_wallet(&self) -> bool {
        false
    }
}

impl SolanaChainProviderLike for SolanaChainProvider {
    async fn simulate_transaction_with_config(
        &self,
        tx: &VersionedTransaction,
        cfg: RpcSimulateTransactionConfig,
    ) -> Result<SimulateTransactionResult, SolanaChainProviderError> {
        let sim = self
            .rpc_client
            .simulate_transaction_with_config(tx, cfg)
            .await?;
        if let Some(err) = sim.value.err {
            return Err(SolanaChainProviderError::InvalidTransaction(err));
        }
        Ok(SimulateTransactionResult {
            inner_instructions: sim.value.inner_instructions.unwrap_or_default(),
            loaded_addresses: sim.value.loaded_addresses.unwrap_or_default(),
            units_consumed: sim.value.units_consumed,
        })
    }

    async fn get_multiple_accounts(
        &self,
        pubkeys: &[Pubkey],
    ) -> Result<Vec<Option<Account>>, SolanaChainProviderError> {
        let accounts = self.rpc_client.get_multiple_accounts(pubkeys).await?;
        Ok(accounts)
    }

    fn max_compute_unit_limit(&self) -> u32 {
        self.max_compute_unit_limit
    }

    fn max_compute_unit_price(&self) -> u64 {
        self.max_compute_unit_price
    }

    fn pubkey(&self) -> Pubkey {
        self.keypair.pubkey()
    }

    fn fee_payer(&self) -> Address {
        Address::new(self.keypair.pubkey())
    }

    fn sign(
        &self,
        tx: VersionedTransaction,
    ) -> Result<VersionedTransaction, SolanaChainProviderError> {
        let mut tx = tx;
        let msg_bytes = tx.message.serialize();
        let signature = self.keypair.try_sign_message(msg_bytes.as_slice())?;
        // Required signatures are the first N account keys
        let num_required = tx.message.header().num_required_signatures as usize;
        let static_keys = tx.message.static_account_keys();
        // Find signer’s position
        #[allow(
            clippy::indexing_slicing,
            reason = "num_required <= static_keys.len() by Solana message invariant"
        )]
        let pos = static_keys[..num_required]
            .iter()
            .position(|k| *k == self.pubkey())
            .ok_or(SolanaChainProviderError::InvalidTransaction(
                UiTransactionError::from(TransactionError::InvalidAccountIndex),
            ))?;
        // Ensure signature vector is large enough, then place the signature
        if tx.signatures.len() < num_required {
            tx.signatures.resize(num_required, Signature::default());
        }
        #[allow(
            clippy::indexing_slicing,
            reason = "pos < num_required, resize ensures len >= num_required"
        )]
        {
            tx.signatures[pos] = signature;
        }
        Ok(tx)
    }

    #[allow(
        clippy::excessive_nesting,
        reason = "pubsub vs polling branches are inherently nested"
    )]
    async fn send_and_confirm(
        &self,
        tx: &VersionedTransaction,
        commitment_config: CommitmentConfig,
    ) -> Result<Signature, SolanaChainProviderError> {
        use futures_util::stream::StreamExt;
        let tx_sig = tx.get_signature();

        if let Some(pubsub_client) = self.pubsub_client.as_ref() {
            let config = RpcSignatureSubscribeConfig {
                commitment: Some(commitment_config),
                enable_received_notification: None,
            };
            let (mut stream, unsubscribe) = pubsub_client
                .signature_subscribe(tx_sig, Some(config))
                .await?;
            if let Err(e) = self.send(tx).await {
                #[cfg(feature = "telemetry")]
                tracing::error!(error = %e, "Failed to send transaction");
                unsubscribe().await;
                return Err(e);
            }
            if let Some(response) = stream.next().await {
                let error = if let RpcSignatureResult::ProcessedSignature(r) = response.value {
                    r.err
                } else {
                    None
                };
                error.map_or(Ok(*tx_sig), |e| {
                    Err(SolanaChainProviderError::InvalidTransaction(e))
                })
            } else {
                Err(SolanaChainProviderError::Transport(Box::new(
                    ClientErrorKind::Custom(
                        "Can not get response from signatureSubscribe".to_owned(),
                    ),
                )))
            }
        } else {
            // Poll for confirmation with a bounded timeout to prevent infinite loops
            // when the transaction never lands (e.g. expired blockhash).
            const MAX_CONFIRM_TIMEOUT: Duration = Duration::from_mins(1);
            const POLL_INTERVAL: Duration = Duration::from_millis(200);

            self.send(tx).await?;
            let deadline = tokio::time::Instant::now() + MAX_CONFIRM_TIMEOUT;
            loop {
                let confirmed = self
                    .rpc_client
                    .confirm_transaction_with_commitment(tx_sig, commitment_config)
                    .await?;
                if confirmed.value {
                    return Ok(*tx_sig);
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(SolanaChainProviderError::Transport(Box::new(
                        ClientErrorKind::Custom(format!(
                            "Transaction confirmation timed out after {MAX_CONFIRM_TIMEOUT:?}"
                        )),
                    )));
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        }
    }

    fn rpc_client(&self) -> Arc<RpcClient> {
        Self::rpc_client(self)
    }

    fn send(
        &self,
        tx: &VersionedTransaction,
    ) -> impl Future<Output = Result<Signature, SolanaChainProviderError>> + Send {
        Self::send(self, tx)
    }

    async fn confirm_signature(
        &self,
        signature: &Signature,
        _commitment_config: CommitmentConfig,
    ) -> Result<SignatureConfirm, SolanaChainProviderError> {
        const MAX_CONFIRM_TIMEOUT: Duration = Duration::from_mins(1);
        const POLL_INTERVAL: Duration = Duration::from_millis(200);
        let deadline = tokio::time::Instant::now() + MAX_CONFIRM_TIMEOUT;
        loop {
            if let Ok(response) = self.rpc_client.get_signature_statuses(&[*signature]).await
                && let Some(outcome) =
                    response
                        .value
                        .first()
                        .and_then(Option::as_ref)
                        .and_then(|entry| {
                            classify_signature_status(
                                entry.confirmation_status.as_ref(),
                                entry.err.clone(),
                            )
                        })
            {
                return Ok(outcome);
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(SignatureConfirm::TimedOut);
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    async fn get_confirmed_inner_instructions(
        &self,
        signature: &Signature,
    ) -> Result<Option<Vec<UiInnerInstructions>>, SolanaChainProviderError> {
        let config = RpcTransactionConfig {
            encoding: Some(UiTransactionEncoding::JsonParsed),
            commitment: Some(CommitmentConfig::confirmed()),
            max_supported_transaction_version: Some(0),
        };
        match self
            .rpc_client
            .get_transaction_with_config(signature, config)
            .await
        {
            Ok(encoded) => Ok(encoded.transaction.meta.and_then(|meta| {
                Option::<Vec<UiInnerInstructions>>::from(meta.inner_instructions)
            })),
            Err(_) => Ok(None),
        }
    }

    async fn get_token_account_balance(
        &self,
        token_account: &Pubkey,
    ) -> Result<Option<u64>, SolanaChainProviderError> {
        match self
            .rpc_client
            .get_token_account_balance_with_commitment(token_account, CommitmentConfig::confirmed())
            .await
        {
            Ok(resp) => Ok(resp.value.amount.parse().ok()),
            Err(_) => Ok(None),
        }
    }

    async fn fetch_address_lookup_tables(
        &self,
        tables: &[Pubkey],
    ) -> Result<HashMap<Pubkey, Vec<Pubkey>>, SolanaChainProviderError> {
        if tables.is_empty() {
            return Ok(HashMap::new());
        }
        let accounts = self.rpc_client.get_multiple_accounts(tables).await?;
        let mut out = HashMap::with_capacity(tables.len());
        for (key, account) in tables.iter().zip(accounts) {
            let Some(account) = account else {
                continue;
            };
            if let Some(addresses) = parse_lookup_table_addresses(&account.data) {
                out.insert(*key, addresses);
            }
        }
        Ok(out)
    }

    fn supports_smart_wallet(&self) -> bool {
        true
    }
}

/// Address Lookup Table: 56-byte meta, then packed 32-byte addresses.
const LOOKUP_TABLE_META_SIZE: usize = 56;

fn parse_lookup_table_addresses(data: &[u8]) -> Option<Vec<Pubkey>> {
    let rest = data.get(LOOKUP_TABLE_META_SIZE..)?;
    let (chunks, tail) = rest.as_chunks::<32>();
    if !tail.is_empty() {
        return None;
    }
    Some(
        chunks
            .iter()
            .map(|chunk| Pubkey::new_from_array(*chunk))
            .collect(),
    )
}

fn classify_signature_status(
    confirmation_status: Option<&TransactionConfirmationStatus>,
    err: Option<TransactionError>,
) -> Option<SignatureConfirm> {
    let committed = matches!(
        confirmation_status,
        Some(TransactionConfirmationStatus::Confirmed | TransactionConfirmationStatus::Finalized)
    );
    if !committed {
        return None;
    }
    Some(err.map_or(SignatureConfirm::Confirmed, |e| {
        SignatureConfirm::OnchainFailure(UiTransactionError::from(e))
    }))
}

impl<T: SolanaChainProviderLike + Send> SolanaChainProviderLike for Arc<T> {
    fn simulate_transaction_with_config(
        &self,
        tx: &VersionedTransaction,
        cfg: RpcSimulateTransactionConfig,
    ) -> impl Future<Output = Result<SimulateTransactionResult, SolanaChainProviderError>> + Send
    {
        (**self).simulate_transaction_with_config(tx, cfg)
    }

    fn get_multiple_accounts(
        &self,
        pubkeys: &[Pubkey],
    ) -> impl Future<Output = Result<Vec<Option<Account>>, SolanaChainProviderError>> + Send {
        (**self).get_multiple_accounts(pubkeys)
    }

    fn max_compute_unit_limit(&self) -> u32 {
        (**self).max_compute_unit_limit()
    }

    fn max_compute_unit_price(&self) -> u64 {
        (**self).max_compute_unit_price()
    }

    fn pubkey(&self) -> Pubkey {
        (**self).pubkey()
    }

    fn fee_payer(&self) -> Address {
        (**self).fee_payer()
    }

    fn sign(
        &self,
        tx: VersionedTransaction,
    ) -> Result<VersionedTransaction, SolanaChainProviderError> {
        (**self).sign(tx)
    }

    fn send_and_confirm(
        &self,
        tx: &VersionedTransaction,
        commitment_config: CommitmentConfig,
    ) -> impl Future<Output = Result<Signature, SolanaChainProviderError>> + Send {
        (**self).send_and_confirm(tx, commitment_config)
    }

    fn rpc_client(&self) -> Arc<RpcClient> {
        (**self).rpc_client()
    }

    fn send(
        &self,
        tx: &VersionedTransaction,
    ) -> impl Future<Output = Result<Signature, SolanaChainProviderError>> + Send {
        (**self).send(tx)
    }

    fn confirm_signature(
        &self,
        signature: &Signature,
        commitment_config: CommitmentConfig,
    ) -> impl Future<Output = Result<SignatureConfirm, SolanaChainProviderError>> + Send {
        (**self).confirm_signature(signature, commitment_config)
    }

    fn get_confirmed_inner_instructions(
        &self,
        signature: &Signature,
    ) -> impl Future<Output = Result<Option<Vec<UiInnerInstructions>>, SolanaChainProviderError>> + Send
    {
        (**self).get_confirmed_inner_instructions(signature)
    }

    fn get_token_account_balance(
        &self,
        token_account: &Pubkey,
    ) -> impl Future<Output = Result<Option<u64>, SolanaChainProviderError>> + Send {
        (**self).get_token_account_balance(token_account)
    }

    fn fetch_address_lookup_tables(
        &self,
        tables: &[Pubkey],
    ) -> impl Future<Output = Result<HashMap<Pubkey, Vec<Pubkey>>, SolanaChainProviderError>> + Send
    {
        (**self).fetch_address_lookup_tables(tables)
    }

    fn supports_smart_wallet(&self) -> bool {
        (**self).supports_smart_wallet()
    }
}

#[cfg(test)]
mod tests {
    use crate::chain::SOLANA_MAX_COMPUTE_UNIT_PRICE_MICROLAMPORTS;

    #[test]
    fn recommended_cu_price_matches_go_cap() {
        assert_eq!(SOLANA_MAX_COMPUTE_UNIT_PRICE_MICROLAMPORTS, 5_000_000);
    }
}
