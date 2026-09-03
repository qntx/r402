//! Facilitator-side Concordium provider: sponsor keys + node.

use std::fmt::{Debug, Formatter};
use std::future::Future;

use concordium_rust_sdk::base::transactions::AccountTransactionV1;
use concordium_rust_sdk::types::transactions::EncodedPayload;
use r402_protocol::network::{ChainId, ChainProvider};

use super::account::{ConcordiumAddress, ConcordiumChainReference};
use super::rpc::{ConcordiumGrpc, ConcordiumNode, ConcordiumRpcError};
use super::signer::ConcordiumSigner;

/// Provider for interacting with Concordium as a facilitator.
#[derive(Clone)]
pub struct ConcordiumChainProvider<N = ConcordiumGrpc> {
    chain: ConcordiumChainReference,
    signers: Vec<ConcordiumSigner>,
    node: N,
}

impl<N: Debug> Debug for ConcordiumChainProvider<N> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConcordiumChainProvider")
            .field("chain", &self.chain)
            .field(
                "signers",
                &self
                    .signers
                    .iter()
                    .map(ConcordiumSigner::address)
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl<N> ConcordiumChainProvider<N> {
    /// Binds `chain` to `signers` and a node handle.
    #[must_use]
    pub const fn new(
        chain: ConcordiumChainReference,
        signers: Vec<ConcordiumSigner>,
        node: N,
    ) -> Self {
        Self {
            chain,
            signers,
            node,
        }
    }

    /// Network this provider is bound to.
    #[must_use]
    pub const fn chain_reference(&self) -> ConcordiumChainReference {
        self.chain
    }

    /// Shared node handle.
    #[must_use]
    pub const fn node(&self) -> &N {
        &self.node
    }

    /// Sponsor signers.
    #[must_use]
    pub fn signers(&self) -> &[ConcordiumSigner] {
        &self.signers
    }

    /// Signer whose address matches `address`.
    #[must_use]
    pub fn signer_for(&self, address: ConcordiumAddress) -> Option<&ConcordiumSigner> {
        self.signers.iter().find(|s| s.address() == address)
    }
}

impl ConcordiumChainProvider<ConcordiumGrpc> {
    /// Connects to the default gRPC endpoint for `chain`.
    ///
    /// # Errors
    ///
    /// gRPC connect failure.
    pub async fn connect(
        chain: ConcordiumChainReference,
        signers: Vec<ConcordiumSigner>,
        grpc_url: Option<String>,
    ) -> Result<Self, ConcordiumRpcError> {
        let url = grpc_url.unwrap_or_else(|| chain.default_grpc_https());
        let node = ConcordiumGrpc::connect(url).await?;
        Ok(Self::new(chain, signers, node))
    }
}

impl<N> ChainProvider for ConcordiumChainProvider<N> {
    fn signer_addresses(&self) -> Vec<String> {
        self.signers
            .iter()
            .map(|s| s.address().to_string())
            .collect()
    }

    fn chain_id(&self) -> ChainId {
        self.chain.into()
    }
}

impl<N: ConcordiumNode> ConcordiumNode for ConcordiumChainProvider<N> {
    fn get_account(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<super::rpc::AccountSnapshot, ConcordiumRpcError>> + Send {
        self.node.get_account(address)
    }

    fn next_nonce(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<u64, ConcordiumRpcError>> + Send {
        self.node.next_nonce(address)
    }

    fn get_token_decimals(
        &self,
        token_id: &str,
    ) -> impl Future<Output = Result<u8, ConcordiumRpcError>> + Send {
        self.node.get_token_decimals(token_id)
    }

    fn get_token_balance(
        &self,
        address: &str,
        token_id: &str,
    ) -> impl Future<Output = Result<Option<u128>, ConcordiumRpcError>> + Send {
        self.node.get_token_balance(address, token_id)
    }

    fn send_v1(
        &self,
        tx: AccountTransactionV1<EncodedPayload>,
    ) -> impl Future<Output = Result<String, ConcordiumRpcError>> + Send {
        self.node.send_v1(tx)
    }

    fn wait_finalized(
        &self,
        tx_hash: &str,
        timeout: std::time::Duration,
    ) -> impl Future<Output = Result<crate::exact::payload::TransactionInfo, ConcordiumRpcError>> + Send
    {
        self.node.wait_finalized(tx_hash, timeout)
    }
}
