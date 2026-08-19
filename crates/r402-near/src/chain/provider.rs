//! Facilitator-side NEAR chain provider: relayer keys + JSON-RPC.

use std::fmt::{Debug, Formatter};
use std::str::FromStr;

use near_crypto::{InMemorySigner, SecretKey, Signer};
use near_jsonrpc_client::methods;
use near_jsonrpc_primitives::types::query::QueryResponseKind;
use near_primitives::action::delegate::SignedDelegateAction;
use near_primitives::hash::CryptoHash;
use near_primitives::transaction::{SignedTransaction, Transaction, TransactionV0};
use near_primitives::types::{AccountId, BlockReference, Finality};
use near_primitives::views::{QueryRequest, TxExecutionStatus};
use r402_core::chain::{ChainId, ChainProvider};

use super::rpc::{
    NearJsonRpc, NearRpc, NearRpcError, NearSettlementOutcome, interpret_settlement_outcome,
};
use super::types::NearChainReference;
use crate::DEFAULT_MAX_SPONSORED_GAS;

/// A relayer account this facilitator controls.
#[derive(Clone)]
pub struct NearRelayer {
    account_id: AccountId,
    signer: Signer,
}

impl Debug for NearRelayer {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NearRelayer")
            .field("account_id", &self.account_id)
            .field("public_key", &self.signer.public_key())
            .finish_non_exhaustive()
    }
}

impl NearRelayer {
    /// Constructs a relayer from an account ID and a NEAR secret key string
    /// (`ed25519:...` or `secp256k1:...`).
    ///
    /// # Errors
    ///
    /// Returns [`NearRpcError::Parse`] if the account ID or secret key is invalid.
    pub fn from_secret_key(
        account_id: impl AsRef<str>,
        secret_key: impl AsRef<str>,
    ) -> Result<Self, NearRpcError> {
        let account_id = AccountId::from_str(account_id.as_ref())
            .map_err(|e| NearRpcError::Parse(e.to_string()))?;
        let secret_key = SecretKey::from_str(secret_key.as_ref())
            .map_err(|e| NearRpcError::Parse(e.to_string()))?;
        let signer = InMemorySigner::from_secret_key(account_id.clone(), secret_key);
        Ok(Self { account_id, signer })
    }

    /// Relayer account ID.
    #[must_use]
    pub const fn account_id(&self) -> &AccountId {
        &self.account_id
    }
}

/// Provider for interacting with a NEAR blockchain as a facilitator.
#[derive(Clone)]
pub struct NearChainProvider {
    chain: NearChainReference,
    relayers: Vec<NearRelayer>,
    rpc: NearJsonRpc,
    max_sponsored_gas: u64,
}

impl Debug for NearChainProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NearChainProvider")
            .field("chain", &self.chain)
            .field(
                "relayers",
                &self
                    .relayers
                    .iter()
                    .map(|r| r.account_id.as_str())
                    .collect::<Vec<_>>(),
            )
            .field("max_sponsored_gas", &self.max_sponsored_gas)
            .finish_non_exhaustive()
    }
}

impl NearChainProvider {
    /// Creates a provider for `chain` using the given relayers and RPC URL.
    ///
    /// When `rpc_url` is `None`, the FastNEAR public endpoint for `chain` is
    /// used.
    #[allow(clippy::doc_markdown, reason = "FastNEAR is a product name")]
    #[must_use]
    pub fn new(
        chain: NearChainReference,
        relayers: Vec<NearRelayer>,
        rpc_url: Option<String>,
    ) -> Self {
        let url = rpc_url.unwrap_or_else(|| chain.default_rpc_url().to_owned());
        Self {
            chain,
            relayers,
            rpc: NearJsonRpc::connect(url),
            max_sponsored_gas: DEFAULT_MAX_SPONSORED_GAS,
        }
    }

    /// Overrides the sponsored-gas cap (gas units).
    #[must_use]
    pub const fn with_max_sponsored_gas(mut self, gas: u64) -> Self {
        self.max_sponsored_gas = gas;
        self
    }

    /// Relayer account IDs managed by this provider.
    #[must_use]
    pub fn relayer_ids(&self) -> Vec<String> {
        self.relayers
            .iter()
            .map(|r| r.account_id.to_string())
            .collect()
    }

    /// Maximum sponsored gas in gas units.
    #[must_use]
    pub const fn max_sponsored_gas(&self) -> u64 {
        self.max_sponsored_gas
    }

    /// The NEAR network this provider is bound to.
    #[must_use]
    pub const fn chain_reference(&self) -> NearChainReference {
        self.chain
    }

    /// Shared JSON-RPC handle.
    #[must_use]
    pub const fn rpc(&self) -> &NearJsonRpc {
        &self.rpc
    }

    /// Submits a signed delegate action through `relayer_id` and waits until
    /// the inner `ft_transfer` receipt has finished executing.
    ///
    /// # Errors
    ///
    /// Returns [`NearRpcError`] when the relayer is unknown, RPC fails, or
    /// the outer transaction cannot be built.
    pub async fn submit_signed_delegate(
        &self,
        relayer_id: &str,
        signed_delegate_b64: &str,
    ) -> Result<NearSettlementOutcome, NearRpcError> {
        let relayer = self
            .relayers
            .iter()
            .find(|r| r.account_id.as_str() == relayer_id)
            .ok_or_else(|| NearRpcError::Parse(format!("unknown_relayer:{relayer_id}")))?;

        let bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            signed_delegate_b64,
        )
        .map_err(|e| NearRpcError::Parse(e.to_string()))?;
        let signed_delegate: SignedDelegateAction = borsh::BorshDeserialize::try_from_slice(&bytes)
            .map_err(|e| NearRpcError::Parse(e.to_string()))?;
        let token_contract_id = signed_delegate.delegate_action.receiver_id.to_string();
        let sender_id = signed_delegate.delegate_action.sender_id.clone();

        let public_key = relayer.signer.public_key();
        let access_key_req = methods::query::RpcQueryRequest {
            block_reference: BlockReference::Finality(Finality::Final),
            request: QueryRequest::ViewAccessKey {
                account_id: relayer.account_id.clone(),
                public_key: public_key.clone(),
            },
        };
        let access_key = self
            .rpc
            .client()
            .call(access_key_req)
            .await
            .map_err(|e| NearRpcError::Rpc(e.to_string()))?;
        let nonce = match access_key.kind {
            QueryResponseKind::AccessKey(view) => view.nonce.saturating_add(1),
            other => {
                return Err(NearRpcError::Parse(format!(
                    "unexpected view_access_key response: {other:?}"
                )));
            }
        };

        let block = self
            .rpc
            .client()
            .call(methods::block::RpcBlockRequest {
                block_reference: BlockReference::Finality(Finality::Final),
            })
            .await
            .map_err(|e| NearRpcError::Rpc(e.to_string()))?;
        let block_hash: CryptoHash = block.header.hash;

        let transaction = Transaction::V0(TransactionV0 {
            signer_id: relayer.account_id.clone(),
            public_key,
            nonce,
            receiver_id: sender_id,
            block_hash,
            actions: vec![near_primitives::action::Action::Delegate(Box::new(
                signed_delegate,
            ))],
        });
        let (hash, _) = transaction.get_hash_and_size();
        let signature = relayer.signer.sign(hash.as_ref());
        let signed_tx = SignedTransaction::new(signature, transaction);

        let response = self
            .rpc
            .client()
            .call(methods::send_tx::RpcSendTransactionRequest {
                signed_transaction: signed_tx,
                wait_until: TxExecutionStatus::Final,
            })
            .await
            .map_err(|e| NearRpcError::Rpc(e.to_string()))?;

        let outcome = response
            .final_execution_outcome
            .ok_or_else(|| NearRpcError::Parse("missing final execution outcome".to_owned()))?
            .into_outcome();

        let receipts: Vec<(String, _)> = outcome
            .receipts_outcome
            .iter()
            .map(|r| (r.outcome.executor_id.to_string(), r.outcome.status.clone()))
            .collect();
        let inner = interpret_settlement_outcome(&outcome.status, &receipts, &token_contract_id);
        Ok(NearSettlementOutcome {
            transaction: outcome.transaction_outcome.id.to_string(),
            inner_receipt: inner,
        })
    }
}

impl ChainProvider for NearChainProvider {
    fn signer_addresses(&self) -> Vec<String> {
        self.relayer_ids()
    }

    fn chain_id(&self) -> ChainId {
        self.chain.into()
    }
}

impl NearRpc for NearChainProvider {
    fn current_block_height(&self) -> impl Future<Output = Result<u64, NearRpcError>> + Send {
        self.rpc.current_block_height()
    }

    fn view_account(
        &self,
        account_id: &str,
    ) -> impl Future<Output = Result<Option<super::rpc::NearAccountView>, NearRpcError>> + Send
    {
        self.rpc.view_account(account_id)
    }

    fn view_access_key(
        &self,
        account_id: &str,
        public_key: &str,
    ) -> impl Future<Output = Result<Option<super::rpc::NearAccessKeyView>, NearRpcError>> + Send
    {
        self.rpc.view_access_key(account_id, public_key)
    }

    fn ft_balance_of(
        &self,
        token: &str,
        account_id: &str,
    ) -> impl Future<Output = Result<u128, NearRpcError>> + Send {
        self.rpc.ft_balance_of(token, account_id)
    }

    fn storage_balance_of(
        &self,
        token: &str,
        account_id: &str,
    ) -> impl Future<Output = Result<super::rpc::NearStorageBalance, NearRpcError>> + Send {
        self.rpc.storage_balance_of(token, account_id)
    }
}
