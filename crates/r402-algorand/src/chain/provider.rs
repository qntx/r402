//! Facilitator-side Algorand chain provider: fee-payer keys + algod.

use std::fmt::{Debug, Formatter};

use r402_protocol::network::{ChainId, ChainProvider};

use super::account::{AlgorandAddress, AlgorandChainReference};
use super::rpc::{
    AlgodClient, AlgodError, AlgodRpc, PendingTransaction, SimulateResult, SuggestedParams,
};
use super::signer::AlgorandSigner;

/// Provider for interacting with Algorand as a facilitator.
#[derive(Clone)]
pub struct AlgorandChainProvider {
    chain: AlgorandChainReference,
    signers: Vec<AlgorandSigner>,
    rpc: AlgodClient,
}

impl Debug for AlgorandChainProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlgorandChainProvider")
            .field("chain", &self.chain)
            .field(
                "signers",
                &self
                    .signers
                    .iter()
                    .map(AlgorandSigner::address)
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl AlgorandChainProvider {
    /// Creates a provider for `chain` using the given fee-payer signers.
    ///
    /// When `algod_url` is `None`, the `AlgoNode` public endpoint for `chain`
    /// is used.
    #[must_use]
    pub fn new(
        chain: AlgorandChainReference,
        signers: Vec<AlgorandSigner>,
        algod_url: Option<String>,
        algod_token: Option<String>,
    ) -> Self {
        let url = algod_url.unwrap_or_else(|| chain.default_algod_url().to_owned());
        Self {
            chain,
            signers,
            rpc: AlgodClient::connect(url, algod_token),
        }
    }

    /// The Algorand network this provider is bound to.
    #[must_use]
    pub const fn chain_reference(&self) -> AlgorandChainReference {
        self.chain
    }

    /// Shared algod handle.
    #[must_use]
    pub const fn rpc(&self) -> &AlgodClient {
        &self.rpc
    }

    /// Signers this facilitator can use as fee payers.
    #[must_use]
    pub fn signers(&self) -> &[AlgorandSigner] {
        &self.signers
    }

    /// Finds a signer whose address matches `sender`.
    #[must_use]
    pub fn signer_for(&self, sender: AlgorandAddress) -> Option<&AlgorandSigner> {
        self.signers.iter().find(|s| s.address() == sender)
    }
}

impl ChainProvider for AlgorandChainProvider {
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

impl AlgodRpc for AlgorandChainProvider {
    fn suggested_params(&self) -> impl Future<Output = Result<SuggestedParams, AlgodError>> + Send {
        self.rpc.suggested_params()
    }

    fn simulate_group(
        &self,
        signed_txns: &[Vec<u8>],
    ) -> impl Future<Output = Result<SimulateResult, AlgodError>> + Send {
        self.rpc.simulate_group(signed_txns)
    }

    fn send_group(
        &self,
        signed_txns: &[Vec<u8>],
    ) -> impl Future<Output = Result<String, AlgodError>> + Send {
        self.rpc.send_group(signed_txns)
    }

    fn pending_transaction(
        &self,
        txid: &str,
    ) -> impl Future<Output = Result<PendingTransaction, AlgodError>> + Send {
        self.rpc.pending_transaction(txid)
    }

    fn last_round(&self) -> impl Future<Output = Result<u64, AlgodError>> + Send {
        self.rpc.last_round()
    }
}
