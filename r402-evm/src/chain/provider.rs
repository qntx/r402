use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use alloy_network::{Ethereum as AlloyEthereum, EthereumWallet, NetworkWallet, TransactionBuilder};
use alloy_primitives::{Address, Bytes};
use alloy_provider::fillers::{
    BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller, WalletFiller,
};
use alloy_provider::{
    Identity, PendingTransactionError, Provider, ProviderBuilder, RootProvider, WalletProvider,
};
use alloy_rpc_client::RpcClient;
use alloy_rpc_types_eth::{BlockId, TransactionReceipt, TransactionRequest};
use alloy_transport::TransportError;
use alloy_transport::layers::{FallbackLayer, ThrottleLayer};
use alloy_transport_http::Http;
use r402::chain::{ChainId, ChainProvider};
use tower::ServiceBuilder;
#[cfg(feature = "telemetry")]
use tracing::Instrument;
use url::Url;

use crate::chain::nonce::PendingNonceManager;
use crate::chain::types::Eip155ChainReference;

/// Combined filler type for gas, blob gas, nonce, and chain ID.
pub type InnerFiller = JoinFill<
    GasFiller,
    JoinFill<BlobGasFiller, JoinFill<NonceFiller<PendingNonceManager>, ChainIdFiller>>,
>;

/// The fully composed Ethereum provider type used in this project.
///
/// Combines multiple filler layers for gas, nonce, chain ID, blob gas, and wallet signing,
/// and wraps a [`RootProvider`] for actual JSON-RPC communication.
pub type InnerProvider = FillProvider<
    JoinFill<JoinFill<Identity, InnerFiller>, WalletFiller<EthereumWallet>>,
    RootProvider,
>;

/// Provider for interacting with EVM-compatible blockchains.
///
/// This provider handles:
/// - Transaction signing with multiple signers (round-robin selection)
/// - Nonce management with automatic reset on failures
/// - Gas estimation and pricing (EIP-1559 and legacy)
/// - Transaction receipt fetching with configurable timeouts
///
/// # Multiple Signers
///
/// The provider supports multiple signers for load distribution. When sending
/// transactions, signers are selected in round-robin fashion to distribute
/// the transaction load and avoid nonce conflicts.
///
/// # Nonce Management
///
/// Uses [`PendingNonceManager`] to track nonces locally and query pending
/// transactions on initialization. If a transaction fails, the nonce is
/// automatically reset to force a fresh query on the next transaction.
#[derive(Debug)]
pub struct Eip155ChainProvider {
    chain: Eip155ChainReference,
    eip1559: bool,
    flashblocks: bool,
    receipt_timeout_secs: u64,
    inner: InnerProvider,
    /// Available signer addresses for round-robin selection.
    signer_addresses: Arc<Vec<Address>>,
    /// Current position in round-robin signer rotation.
    signer_cursor: Arc<AtomicUsize>,
    /// Nonce manager for resetting nonces on transaction failures.
    nonce_manager: PendingNonceManager,
}

impl Eip155ChainProvider {
    /// Creates an RPC client from HTTP endpoint URLs with optional per-endpoint rate limits.
    ///
    /// Each entry in `endpoints` is a `(url, optional_rate_limit)` pair.
    /// Non-HTTP(S) URLs are silently skipped.
    ///
    /// # Panics
    ///
    /// Panics if no valid HTTP transports remain after filtering.
    #[allow(unused_variables)] // chain_id is needed for tracing only
    #[must_use]
    pub fn rpc_client(chain_id: &ChainId, endpoints: &[(Url, Option<u32>)]) -> RpcClient {
        let transports = endpoints
            .iter()
            .filter_map(|(url, rate_limit)| {
                let scheme = url.scheme();
                let is_http = scheme == "http" || scheme == "https";
                if !is_http {
                    return None;
                }
                #[cfg(feature = "telemetry")]
                tracing::info!(chain=%chain_id, rpc_url=%url, rate_limit=?rate_limit, "Using HTTP transport");
                let limit = rate_limit.unwrap_or(u32::MAX);
                let service = ServiceBuilder::new()
                    .layer(ThrottleLayer::new(limit))
                    .service(Http::new(url.clone()));
                Some(service)
            })
            .collect::<Vec<_>>();
        let fallback = ServiceBuilder::new()
            .layer(
                FallbackLayer::default().with_active_transport_count(
                    NonZeroUsize::new(transports.len())
                        .expect("Non-zero amount of stateless transports"),
                ),
            )
            .service(transports);
        RpcClient::new(fallback, false)
    }

    /// Creates a new EVM chain provider.
    ///
    /// # Parameters
    ///
    /// - `chain`: The numeric chain reference (e.g., 8453 for Base)
    /// - `wallet`: A pre-built Ethereum wallet containing one or more signers
    /// - `rpc_endpoints`: HTTP RPC endpoints as `(url, optional_rate_limit)` pairs
    /// - `eip1559`: Whether the chain supports EIP-1559 gas pricing
    /// - `flashblocks`: Whether the chain supports flashblocks
    /// - `receipt_timeout_secs`: How long to wait for a transaction receipt
    ///
    /// # Errors
    ///
    /// Returns an error if the wallet has no signers.
    pub fn new(
        chain: Eip155ChainReference,
        wallet: EthereumWallet,
        rpc_endpoints: &[(Url, Option<u32>)],
        eip1559: bool,
        flashblocks: bool,
        receipt_timeout_secs: u64,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let signer_addresses =
            NetworkWallet::<AlloyEthereum>::signer_addresses(&wallet).collect::<Vec<_>>();
        if signer_addresses.is_empty() {
            return Err("at least one signer must be provided".into());
        }
        let signer_addresses = Arc::new(signer_addresses);
        let signer_cursor = Arc::new(AtomicUsize::new(0));

        let chain_id: ChainId = chain.into();
        let client = Self::rpc_client(&chain_id, rpc_endpoints);

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
        let inner: InnerProvider = ProviderBuilder::default()
            .filler(filler)
            .wallet(wallet)
            .connect_client(client);

        #[cfg(feature = "telemetry")]
        tracing::info!(chain=%chain_id, signers=?signer_addresses, "Using EVM provider");

        Ok(Self {
            chain,
            eip1559,
            flashblocks,
            receipt_timeout_secs,
            inner,
            signer_addresses,
            signer_cursor,
            nonce_manager,
        })
    }

    /// Round-robin selection of next signer from wallet.
    fn next_signer_address(&self) -> Address {
        debug_assert!(!self.signer_addresses.is_empty());
        if self.signer_addresses.len() == 1 {
            self.signer_addresses[0]
        } else {
            let next =
                self.signer_cursor.fetch_add(1, Ordering::Relaxed) % self.signer_addresses.len();
            self.signer_addresses[next]
        }
    }
}

/// Errors that can occur when sending a meta-transaction.
#[derive(Debug, thiserror::Error)]
pub enum MetaTransactionSendError {
    /// RPC transport error.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// Pending transaction error.
    #[error(transparent)]
    PendingTransaction(#[from] PendingTransactionError),
    /// Custom error message.
    #[error("{0}")]
    Custom(String),
}

/// Meta-transaction parameters: target address, calldata, and required confirmations.
#[derive(Debug)]
pub struct MetaTransaction {
    /// Target contract address.
    pub to: Address,
    /// Transaction calldata (encoded function call).
    pub calldata: Bytes,
    /// Number of block confirmations to wait for.
    pub confirmations: u64,
}

impl ChainProvider for Eip155ChainProvider {
    fn signer_addresses(&self) -> Vec<String> {
        self.inner
            .signer_addresses()
            .map(|a| a.to_string())
            .collect()
    }

    fn chain_id(&self) -> ChainId {
        self.chain.into()
    }
}

/// Trait for sending meta-transactions with custom target and calldata.
pub trait Eip155MetaTransactionProvider {
    /// Error type for operations.
    type Error;
    /// Underlying provider type.
    type Inner: Provider;

    /// Returns reference to underlying provider.
    fn inner(&self) -> &Self::Inner;
    /// Returns reference to chain descriptor.
    fn chain(&self) -> &Eip155ChainReference;

    /// Sends a meta-transaction to the network.
    fn send_transaction(
        &self,
        tx: MetaTransaction,
    ) -> impl Future<Output = Result<TransactionReceipt, Self::Error>> + Send;
}

impl<T: Eip155MetaTransactionProvider> Eip155MetaTransactionProvider for Arc<T> {
    type Error = T::Error;
    type Inner = T::Inner;

    fn inner(&self) -> &Self::Inner {
        (**self).inner()
    }

    fn chain(&self) -> &Eip155ChainReference {
        (**self).chain()
    }

    fn send_transaction(
        &self,
        tx: MetaTransaction,
    ) -> impl Future<Output = Result<TransactionReceipt, Self::Error>> + Send {
        (**self).send_transaction(tx)
    }
}

impl Eip155MetaTransactionProvider for Eip155ChainProvider {
    type Error = MetaTransactionSendError;
    type Inner = InnerProvider;

    fn inner(&self) -> &Self::Inner {
        &self.inner
    }

    fn chain(&self) -> &Eip155ChainReference {
        &self.chain
    }

    /// Send a meta-transaction with provided `to`, `calldata`, and automatically selected signer.
    ///
    /// This method constructs a transaction from the provided [`MetaTransaction`], automatically
    /// selects the next available signer using round-robin selection, and handles gas pricing
    /// based on whether the network supports EIP-1559.
    ///
    /// If the transaction fails at any point (during submission or receipt fetching), the nonce
    /// for the sending address is reset to force a fresh query on the next transaction. This
    /// ensures correctness even when transactions partially succeed (e.g., submitted but receipt
    /// fetch times out).
    ///
    /// # Gas Pricing Strategy
    ///
    /// - **EIP-1559 networks**: Uses automatic gas pricing via the provider's fillers.
    /// - **Legacy networks**: Fetches the current gas price using `get_gas_price()` and sets it explicitly.
    ///
    /// # Timeout Configuration
    ///
    /// Receipt fetching is subject to a configurable timeout:
    /// - Default: 30 seconds
    /// - Override via `TX_RECEIPT_TIMEOUT_SECS` environment variable
    /// - If the timeout expires, the nonce is reset and an error is returned
    ///
    /// # Parameters
    ///
    /// - `tx`: A [`MetaTransaction`] containing the target address and calldata.
    ///
    /// # Returns
    ///
    /// A [`TransactionReceipt`] once the transaction has been mined and confirmed.
    ///
    /// # Errors
    ///
    /// Returns `FacilitatorLocalError::ContractCall` if:
    /// - Gas price fetching fails (on legacy networks)
    /// - Transaction sending fails
    /// - Receipt retrieval fails or times out
    async fn send_transaction(
        &self,
        tx: MetaTransaction,
    ) -> Result<TransactionReceipt, Self::Error> {
        let from_address = self.next_signer_address();
        let mut txr = TransactionRequest::default()
            .with_to(tx.to)
            .with_from(from_address)
            .with_input(tx.calldata);

        if !self.eip1559 {
            let provider = &self.inner;
            let gas_fut = provider.get_gas_price();
            #[cfg(feature = "telemetry")]
            let gas: u128 = gas_fut
                .instrument(tracing::info_span!("get_gas_price"))
                .await?;
            #[cfg(not(feature = "telemetry"))]
            let gas: u128 = gas_fut.await?;
            txr.set_gas_price(gas);
        }

        // Estimate gas if not provided
        if txr.gas.is_none() {
            let block_id = if self.flashblocks {
                BlockId::latest()
            } else {
                BlockId::pending()
            };
            let gas_limit = self.inner.estimate_gas(txr.clone()).block(block_id).await?;
            txr.set_gas_limit(gas_limit);
        }

        // Send transaction with error handling for nonce reset
        let pending_tx = match self.inner.send_transaction(txr).await {
            Ok(pending) => pending,
            Err(e) => {
                // Transaction submission failed - reset nonce to force requery
                self.nonce_manager.reset_nonce(from_address).await;
                return Err(MetaTransactionSendError::Transport(e));
            }
        };

        // Get receipt with timeout and error handling for nonce reset
        // Default timeout of 30 seconds is reasonable for most EVM chains
        let timeout = std::time::Duration::from_secs(self.receipt_timeout_secs);

        let watcher = pending_tx
            .with_required_confirmations(tx.confirmations)
            .with_timeout(Some(timeout));

        match watcher.get_receipt().await {
            Ok(receipt) => Ok(receipt),
            Err(e) => {
                // Receipt fetch failed (timeout or other error) - reset nonce to force requery
                self.nonce_manager.reset_nonce(from_address).await;
                Err(MetaTransactionSendError::PendingTransaction(e))
            }
        }
    }
}
