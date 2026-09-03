//! Facilitator-side XRPL chain provider: JSON-RPC only (payer pays fees).

use std::fmt::{Debug, Formatter};

use r402_protocol::network::{ChainId, ChainProvider};

use super::account::XrplChainReference;
use super::rpc::{XrplJsonRpc, XrplRpc, XrplRpcError};
use crate::DEFAULT_MAX_FEE_DROPS;

/// Provider for interacting with an XRPL network as a facilitator.
///
/// The payer signs and pays the transaction fee; this provider has no hot
/// wallet. [`ChainProvider::signer_addresses`] is always empty.
#[derive(Clone)]
pub struct XrplChainProvider {
    chain: XrplChainReference,
    rpc: XrplJsonRpc,
    max_fee_drops: u64,
}

impl Debug for XrplChainProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XrplChainProvider")
            .field("chain", &self.chain)
            .field("max_fee_drops", &self.max_fee_drops)
            .finish_non_exhaustive()
    }
}

impl XrplChainProvider {
    /// Creates a provider for `chain`.
    ///
    /// When `rpc_url` is `None`, the default public JSON-RPC URL for `chain`
    /// is used (mainnet / testnet / devnet only).
    ///
    /// # Errors
    ///
    /// Returns [`XrplRpcError::Parse`] when `chain` has no default URL and
    /// `rpc_url` is omitted.
    pub fn new(chain: XrplChainReference, rpc_url: Option<String>) -> Result<Self, XrplRpcError> {
        Ok(Self {
            chain,
            rpc: XrplJsonRpc::connect_chain(chain, rpc_url)?,
            max_fee_drops: DEFAULT_MAX_FEE_DROPS,
        })
    }

    /// Overrides the maximum accepted transaction fee in drops.
    #[must_use]
    pub const fn with_max_fee_drops(mut self, drops: u64) -> Self {
        self.max_fee_drops = drops;
        self
    }

    /// Maximum accepted fee in drops.
    #[must_use]
    pub const fn max_fee_drops(&self) -> u64 {
        self.max_fee_drops
    }

    /// Network this provider is bound to.
    #[must_use]
    pub const fn chain_reference(&self) -> XrplChainReference {
        self.chain
    }

    /// Shared JSON-RPC handle.
    #[must_use]
    pub const fn rpc(&self) -> &XrplJsonRpc {
        &self.rpc
    }
}

impl ChainProvider for XrplChainProvider {
    fn signer_addresses(&self) -> Vec<String> {
        Vec::new()
    }

    fn chain_id(&self) -> ChainId {
        self.chain.into()
    }
}

impl XrplRpc for XrplChainProvider {
    fn current_ledger_index(&self) -> impl Future<Output = Result<u32, XrplRpcError>> + Send {
        self.rpc.current_ledger_index()
    }

    fn account_authorization(
        &self,
        account: &str,
    ) -> impl Future<Output = Result<super::rpc::XrplAccountAuthorization, XrplRpcError>> + Send
    {
        self.rpc.account_authorization(account)
    }

    fn ticket_sequences(
        &self,
        account: &str,
    ) -> impl Future<Output = Result<Vec<u32>, XrplRpcError>> + Send {
        self.rpc.ticket_sequences(account)
    }

    fn fee_drops(&self) -> impl Future<Output = Result<u64, XrplRpcError>> + Send {
        self.rpc.fee_drops()
    }

    fn simulate(
        &self,
        unsigned_tx: &serde_json::Value,
    ) -> impl Future<Output = Result<super::rpc::XrplSimulationResult, XrplRpcError>> + Send {
        self.rpc.simulate(unsigned_tx)
    }

    fn submit(
        &self,
        signed_tx_blob: &str,
    ) -> impl Future<Output = Result<super::rpc::XrplSubmitResult, XrplRpcError>> + Send {
        self.rpc.submit(signed_tx_blob)
    }

    fn tx(
        &self,
        hash: &str,
    ) -> impl Future<Output = Result<Option<super::rpc::XrplTxResult>, XrplRpcError>> + Send {
        self.rpc.tx(hash)
    }
}
