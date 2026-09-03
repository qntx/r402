//! Facilitator-side Keeta chain provider: fee-payer keys + read client.

use std::fmt::{Debug, Formatter};
use std::str::FromStr;
use std::sync::Arc;

use keetanetwork_account::{
    Account, Accountable, GenericAccount, KeyED25519, KeyPairType, Keyable,
};
use keetanetwork_block::{AccountRef, Amount, BaseFlag, BlockHash};
use keetanetwork_client::{ClientError as KeetaClientError, UserClient};
use keetanetwork_crypto::prelude::IntoSecret;
use r402_protocol::network::{ChainId, ChainProvider};

use super::account::{KeetaChainReference, account_has_private_key};

/// A fee-payer account this facilitator controls.
#[derive(Clone)]
pub struct KeetaFeePayer {
    account: AccountRef,
}

impl Debug for KeetaFeePayer {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeetaFeePayer")
            .field("address", &self.account.to_string())
            .finish_non_exhaustive()
    }
}

impl KeetaFeePayer {
    /// Wraps an account that must hold a private key.
    ///
    /// # Errors
    ///
    /// Returns [`KeetaRpcError::Parse`] when the account cannot sign.
    pub fn from_account(account: AccountRef) -> Result<Self, KeetaRpcError> {
        if !account_has_private_key(&account) {
            return Err(KeetaRpcError::Parse(format!(
                "fee payer {account} has no private key"
            )));
        }
        Ok(Self { account })
    }

    /// Derives an Ed25519 fee payer from a 32-byte seed and derivation index.
    ///
    /// # Errors
    ///
    /// Returns [`KeetaRpcError::Parse`] when key derivation fails.
    pub fn from_ed25519_seed(seed: [u8; 32], index: u32) -> Result<Self, KeetaRpcError> {
        let account = Account::<KeyED25519>::try_from(Accountable::KeyAndType(
            Keyable::Seed((seed.into_secret(), index)),
            KeyPairType::ED25519,
        ))
        .map_err(|e| KeetaRpcError::Parse(e.to_string()))?;
        Self::from_account(Arc::new(GenericAccount::Ed25519(account)))
    }

    /// Fee-payer address (`keeta_…`).
    #[must_use]
    pub fn address(&self) -> String {
        self.account.to_string()
    }

    /// Shared account handle used for signing.
    #[must_use]
    pub const fn account(&self) -> &AccountRef {
        &self.account
    }
}

/// Provider for interacting with a Keeta network as a facilitator.
#[derive(Clone)]
pub struct KeetaChainProvider {
    inner: Arc<KeetaChainProviderInner>,
}

struct KeetaChainProviderInner {
    chain: KeetaChainReference,
    fee_payers: Vec<KeetaFeePayer>,
    reader: UserClient,
}

impl Drop for KeetaChainProviderInner {
    fn drop(&mut self) {
        self.reader.client().destroy();
    }
}

impl Debug for KeetaChainProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeetaChainProvider")
            .field("chain", &self.inner.chain)
            .field(
                "fee_payers",
                &self
                    .inner
                    .fee_payers
                    .iter()
                    .map(KeetaFeePayer::address)
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl KeetaChainProvider {
    /// Creates a provider for `chain` using the given fee payers.
    ///
    /// Constructs a read-only [`UserClient`] against the network's default
    /// representatives.
    ///
    /// # Errors
    ///
    /// Returns [`KeetaRpcError`] when the network registry cannot be loaded.
    pub fn new(
        chain: KeetaChainReference,
        fee_payers: Vec<KeetaFeePayer>,
    ) -> Result<Self, KeetaRpcError> {
        let reader = UserClient::from_network(chain.client_network(), None)
            .map_err(|e| KeetaRpcError::Rpc(e.to_string()))?;
        Ok(Self {
            inner: Arc::new(KeetaChainProviderInner {
                chain,
                fee_payers,
                reader,
            }),
        })
    }

    /// Fee-payer addresses managed by this provider.
    #[must_use]
    pub fn fee_payer_ids(&self) -> Vec<String> {
        self.inner
            .fee_payers
            .iter()
            .map(KeetaFeePayer::address)
            .collect()
    }

    /// Fee-payer accounts managed by this provider.
    #[must_use]
    pub fn fee_payers(&self) -> &[KeetaFeePayer] {
        &self.inner.fee_payers
    }

    /// The Keeta network this provider is bound to.
    #[must_use]
    pub fn chain_reference(&self) -> KeetaChainReference {
        self.inner.chain
    }

    /// Read-only user client used for preflight.
    #[must_use]
    pub fn reader(&self) -> &UserClient {
        &self.inner.reader
    }
}

/// Errors from Keeta preflight reads.
#[derive(Debug, thiserror::Error)]
pub enum KeetaRpcError {
    /// Transport or node failure.
    #[error("keeta rpc error: {0}")]
    Rpc(String),
    /// Address or key material could not be parsed.
    #[error("keeta parse error: {0}")]
    Parse(String),
}

/// Flattened ACL used by exact verify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeetaAclView {
    /// Entity the grant applies to (`block.account`).
    pub entity: String,
    /// `OWNER` base flag (implies every other grant).
    pub owner: bool,
    /// `SEND_ON_BEHALF` base flag.
    pub send_on_behalf: bool,
}

/// Read-only Keeta operations used by verify preflight.
pub trait KeetaPreflight: Send + Sync {
    /// Settled balance of `token` held by `account`.
    fn balance(
        &self,
        account: &str,
        token: &str,
    ) -> impl Future<Output = Result<Amount, KeetaRpcError>> + Send;

    /// Head block hash of `account`, or `None` when the account has no blocks.
    fn head_hash(
        &self,
        account: &str,
    ) -> impl Future<Output = Result<Option<BlockHash>, KeetaRpcError>> + Send;

    /// Access-control entries granted by `signer` as principal.
    fn acls_for_signer(
        &self,
        signer: &str,
    ) -> impl Future<Output = Result<Vec<KeetaAclView>, KeetaRpcError>> + Send;
}

impl ChainProvider for KeetaChainProvider {
    fn signer_addresses(&self) -> Vec<String> {
        self.fee_payer_ids()
    }

    fn chain_id(&self) -> ChainId {
        self.inner.chain.into()
    }
}

impl KeetaPreflight for KeetaChainProvider {
    async fn balance(&self, account: &str, token: &str) -> Result<Amount, KeetaRpcError> {
        let account = parse_ref(account)?;
        let token = parse_ref(token)?;
        self.inner
            .reader
            .client()
            .balance(&*account, &*token)
            .await
            .map_err(|e| rpc_err(&e))
    }

    async fn head_hash(&self, account: &str) -> Result<Option<BlockHash>, KeetaRpcError> {
        let account = parse_ref(account)?;
        let state = self
            .inner
            .reader
            .client()
            .state(&*account)
            .await
            .map_err(|e| rpc_err(&e))?;
        Ok(state.head)
    }

    async fn acls_for_signer(&self, signer: &str) -> Result<Vec<KeetaAclView>, KeetaRpcError> {
        let signer = parse_ref(signer)?;
        let acls = self
            .inner
            .reader
            .client()
            .acls_by_principal(&*signer)
            .await
            .map_err(|e| rpc_err(&e))?;
        Ok(acls
            .into_iter()
            .filter_map(|acl| {
                let entity = acl.entity.as_ref()?.to_string();
                Some(KeetaAclView {
                    entity,
                    owner: acl.permissions.has(&[BaseFlag::Owner], &[]),
                    send_on_behalf: acl.permissions.has(&[BaseFlag::SendOnBehalf], &[]),
                })
            })
            .collect())
    }
}

fn parse_ref(s: &str) -> Result<AccountRef, KeetaRpcError> {
    let account = GenericAccount::from_str(s).map_err(|e| KeetaRpcError::Parse(e.to_string()))?;
    Ok(Arc::new(account))
}

fn rpc_err(err: &KeetaClientError) -> KeetaRpcError {
    KeetaRpcError::Rpc(err.to_string())
}
