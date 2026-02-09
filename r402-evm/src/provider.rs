//! EVM chain provider with production-grade settlement capabilities.
//!
//! Provides [`Eip155ChainProvider`] with:
//! - Full filler stack (gas, blob gas, nonce, chain ID, wallet)
//! - [`PendingNonceManager`] for concurrent nonce tracking with pending queries
//! - Multiple signer support with round-robin selection
//! - Automatic nonce reset on transaction failures
//! - Configurable EIP-1559/legacy gas, flashblocks, receipt timeouts

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use alloy_network::{Ethereum, EthereumWallet, Network, NetworkWallet, TransactionBuilder};
use alloy_primitives::{Address, Bytes};
use alloy_provider::fillers::{
    BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller, NonceManager,
    WalletFiller,
};
use alloy_provider::{Identity, PendingTransactionError, Provider, ProviderBuilder, RootProvider};
use alloy_rpc_client::RpcClient;
use alloy_rpc_types_eth::{BlockId, TransactionReceipt, TransactionRequest};
use alloy_transport::{TransportError, TransportResult};
use dashmap::DashMap;
use futures::lock::Mutex;

/// Nonce manager that queries pending transactions for initial nonce.
///
/// Unlike alloy's default [`CachedNonceManager`] which uses the `latest`
/// transaction count, this manager queries with `.pending()` on first use,
/// which includes transactions still in the mempool. This prevents
/// "nonce too low" errors when the application restarts while transactions
/// are still pending.
///
/// - **First call per address**: queries with `.pending()` from RPC
/// - **Subsequent calls**: increments cached nonce locally
/// - **On failure**: [`reset_nonce`](Self::reset_nonce) forces re-query
#[derive(Clone, Debug, Default)]
pub struct PendingNonceManager {
    nonces: Arc<DashMap<Address, Arc<Mutex<u64>>>>,
}

const NONCE_UNSET: u64 = u64::MAX;

#[cfg_attr(target_family = "wasm", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait::async_trait)]
impl NonceManager for PendingNonceManager {
    async fn get_next_nonce<P, N>(&self, provider: &P, address: Address) -> TransportResult<u64>
    where
        P: Provider<N>,
        N: Network,
    {
        let slot = {
            let entry = self
                .nonces
                .entry(address)
                .or_insert_with(|| Arc::new(Mutex::new(NONCE_UNSET)));
            Arc::clone(entry.value())
        };

        let mut nonce = slot.lock().await;
        let new_nonce = if *nonce == NONCE_UNSET {
            provider.get_transaction_count(address).pending().await?
        } else {
            *nonce + 1
        };
        *nonce = new_nonce;
        Ok(new_nonce)
    }
}

impl PendingNonceManager {
    /// Resets the cached nonce for an address, forcing a fresh RPC query
    /// on next use.
    ///
    /// Call this when a transaction fails, as the on-chain state may be
    /// uncertain (the transaction may or may not have reached the mempool).
    pub async fn reset_nonce(&self, address: Address) {
        if let Some(nonce_lock) = self.nonces.get(&address) {
            let mut nonce = nonce_lock.lock().await;
            *nonce = NONCE_UNSET;
        }
    }
}

/// Parameters for an on-chain settlement transaction.
#[derive(Debug, Clone)]
pub struct MetaTransaction {
    /// Target contract address.
    pub to: Address,
    /// Encoded function call data.
    pub calldata: Bytes,
    /// Number of block confirmations to wait for (typically 1).
    pub confirmations: u64,
}

/// Errors that can occur during settlement transactions.
#[derive(Debug, thiserror::Error)]
pub enum SettlementError {
    /// RPC transport or transaction submission error.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// Error waiting for transaction receipt (timeout, etc.).
    #[error(transparent)]
    PendingTransaction(#[from] PendingTransactionError),
}

/// Trait for EVM providers capable of on-chain settlement.
///
/// Separates read operations (via [`read_provider`](Self::read_provider))
/// from write operations (via [`send_transaction`](Self::send_transaction))
/// to enable nonce management, signer rotation, and error recovery.
pub trait EvmSettlementProvider: Send + Sync + 'static {
    /// The underlying read-only provider type.
    type ReadProvider: Provider + Send + Sync;
    /// Error type for settlement operations.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Returns a reference to the provider for read/call operations.
    fn read_provider(&self) -> &Self::ReadProvider;

    /// Returns the next signer address (may rotate among multiple signers).
    fn signer_address(&self) -> Address;

    /// Returns all configured signer addresses.
    fn signer_addresses(&self) -> Vec<Address>;

    /// Sends a settlement transaction with automatic nonce management
    /// and error recovery (nonce reset on failure).
    fn send_transaction(
        &self,
        tx: MetaTransaction,
    ) -> impl Future<Output = Result<TransactionReceipt, Self::Error>> + Send;
}

/// Combined filler type: Gas + `BlobGas` + Nonce([`PendingNonceManager`]) + `ChainId`.
pub type InnerFiller = JoinFill<
    GasFiller,
    JoinFill<BlobGasFiller, JoinFill<NonceFiller<PendingNonceManager>, ChainIdFiller>>,
>;

/// Fully composed Ethereum provider with all fillers and wallet signing.
pub type FullProvider = FillProvider<
    JoinFill<JoinFill<Identity, InnerFiller>, WalletFiller<EthereumWallet>>,
    RootProvider,
>;

/// Configuration for [`Eip155ChainProvider`].
#[derive(Debug, Clone, Copy)]
pub struct ChainProviderConfig {
    /// Whether the chain supports EIP-1559 gas pricing (default: `true`).
    pub eip1559: bool,
    /// Whether the chain uses flashblocks for immediate finality (default: `false`).
    pub flashblocks: bool,
    /// Seconds to wait for a transaction receipt (default: 30).
    pub receipt_timeout_secs: u64,
}

impl Default for ChainProviderConfig {
    fn default() -> Self {
        Self {
            eip1559: true,
            flashblocks: false,
            receipt_timeout_secs: 30,
        }
    }
}

/// Production-grade EVM chain provider with nonce management and signer rotation.
///
/// Wraps a fully-composed alloy provider with:
/// - [`GasFiller`] + [`BlobGasFiller`] for automatic gas estimation
/// - [`NonceFiller`] with [`PendingNonceManager`] for concurrent nonce tracking
/// - [`ChainIdFiller`] for automatic chain ID
/// - [`WalletFiller`] for transaction signing
/// - Round-robin signer selection for load distribution
/// - Automatic nonce reset on transaction failures
#[derive(Debug)]
pub struct Eip155ChainProvider {
    inner: FullProvider,
    eip1559: bool,
    flashblocks: bool,
    receipt_timeout_secs: u64,
    signer_addrs: Arc<Vec<Address>>,
    signer_cursor: Arc<AtomicUsize>,
    nonce_manager: PendingNonceManager,
}

impl Eip155ChainProvider {
    /// Creates a new provider from a pre-built RPC client and wallet.
    ///
    /// The `rpc_client` should already be configured with transport-level
    /// concerns (timeouts, fallback, rate limiting). The `wallet` should
    /// contain all signers for this chain.
    #[must_use]
    pub fn new(rpc_client: RpcClient, wallet: EthereumWallet, config: ChainProviderConfig) -> Self {
        let signer_addrs: Vec<Address> =
            NetworkWallet::<Ethereum>::signer_addresses(&wallet).collect();
        let signer_addrs = Arc::new(signer_addrs);
        let nonce_manager = PendingNonceManager::default();

        let filler = JoinFill::new(
            GasFiller,
            JoinFill::new(
                BlobGasFiller::default(),
                JoinFill::new(
                    NonceFiller::new(nonce_manager.clone()),
                    ChainIdFiller::default(),
                ),
            ),
        );

        let inner: FullProvider = ProviderBuilder::default()
            .filler(filler)
            .wallet(wallet)
            .connect_client(rpc_client);

        Self {
            inner,
            eip1559: config.eip1559,
            flashblocks: config.flashblocks,
            receipt_timeout_secs: config.receipt_timeout_secs,
            signer_addrs,
            signer_cursor: Arc::new(AtomicUsize::new(0)),
            nonce_manager,
        }
    }

    /// Selects the next signer address using round-robin rotation.
    fn next_signer(&self) -> Address {
        if self.signer_addrs.len() == 1 {
            self.signer_addrs[0]
        } else {
            let idx = self.signer_cursor.fetch_add(1, Ordering::Relaxed) % self.signer_addrs.len();
            self.signer_addrs[idx]
        }
    }
}

impl EvmSettlementProvider for Eip155ChainProvider {
    type ReadProvider = FullProvider;
    type Error = SettlementError;

    fn read_provider(&self) -> &FullProvider {
        &self.inner
    }

    fn signer_address(&self) -> Address {
        self.next_signer()
    }

    fn signer_addresses(&self) -> Vec<Address> {
        self.signer_addrs.as_ref().clone()
    }

    async fn send_transaction(
        &self,
        tx: MetaTransaction,
    ) -> Result<TransactionReceipt, SettlementError> {
        let from_address = self.next_signer();

        let mut txr = TransactionRequest::default()
            .with_to(tx.to)
            .with_from(from_address)
            .with_input(tx.calldata);

        // Legacy gas pricing for non-EIP-1559 chains
        if !self.eip1559 {
            let gas = self.inner.get_gas_price().await?;
            txr.set_gas_price(gas);
        }

        // Estimate gas
        let block_id = if self.flashblocks {
            BlockId::latest()
        } else {
            BlockId::pending()
        };
        let gas_limit = self.inner.estimate_gas(txr.clone()).block(block_id).await?;
        txr.set_gas_limit(gas_limit);

        // Send transaction with nonce reset on failure
        let pending_tx = match self.inner.send_transaction(txr).await {
            Ok(pending) => pending,
            Err(e) => {
                self.nonce_manager.reset_nonce(from_address).await;
                return Err(SettlementError::Transport(e));
            }
        };

        // Wait for receipt with timeout
        let timeout = std::time::Duration::from_secs(self.receipt_timeout_secs);
        let watcher = pending_tx
            .with_required_confirmations(tx.confirmations)
            .with_timeout(Some(timeout));

        match watcher.get_receipt().await {
            Ok(receipt) => Ok(receipt),
            Err(e) => {
                self.nonce_manager.reset_nonce(from_address).await;
                Err(SettlementError::PendingTransaction(e))
            }
        }
    }
}
