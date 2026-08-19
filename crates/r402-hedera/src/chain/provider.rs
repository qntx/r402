//! Facilitator-side Hedera provider: fee-payer keys + Mirror REST + Hiero submit.

use std::fmt::{Debug, Formatter};
use std::str::FromStr;

use base64::Engine;
use hedera::{AccountId, AnyTransaction, PrivateKey, TransferTransaction};
use r402_core::chain::{ChainId, ChainProvider};

use super::rpc::{
    HederaAccountResolution, HederaMirrorClient, HederaMirrorError, HederaPreflightResult,
    HederaSignatureResult, preflight_transfer, resolve_account, verify_payer_signature,
};
use super::tx::create_hedera_client;
use super::types::HederaChainReference;

/// A fee-payer account this facilitator controls.
#[derive(Clone)]
pub struct HederaFeePayer {
    account_id: AccountId,
    private_key: PrivateKey,
}

impl Debug for HederaFeePayer {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HederaFeePayer")
            .field("account_id", &self.account_id)
            .field("public_key", &self.private_key.public_key())
            .finish_non_exhaustive()
    }
}

impl HederaFeePayer {
    /// Constructs a fee payer from an account id and a hex-encoded private key.
    ///
    /// # Errors
    ///
    /// Returns [`HederaProviderError::Parse`] if the account id or key is invalid.
    pub fn from_secret(
        account_id: impl AsRef<str>,
        private_key: impl AsRef<str>,
    ) -> Result<Self, HederaProviderError> {
        let account_id = AccountId::from_str(account_id.as_ref())
            .map_err(|e| HederaProviderError::Parse(e.to_string()))?;
        let private_key = PrivateKey::from_str(private_key.as_ref())
            .map_err(|e| HederaProviderError::Parse(e.to_string()))?;
        Ok(Self {
            account_id,
            private_key,
        })
    }

    /// Constructs a fee payer from already-parsed Hiero types.
    #[must_use]
    #[allow(
        clippy::large_types_passed_by_value,
        reason = "Hiero AccountId is Copy but includes optional key material"
    )]
    pub const fn new(account_id: AccountId, private_key: PrivateKey) -> Self {
        Self {
            account_id,
            private_key,
        }
    }

    /// Fee-payer account id.
    #[must_use]
    pub const fn account_id(&self) -> AccountId {
        self.account_id
    }
}

/// Errors from the Hedera facilitator provider.
#[derive(Debug, thiserror::Error)]
pub enum HederaProviderError {
    /// Identifier or key could not be parsed.
    #[error("hedera provider parse error: {0}")]
    Parse(String),
    /// Mirror Node request failed.
    #[error(transparent)]
    Mirror(#[from] HederaMirrorError),
    /// Settlement execute / receipt failed.
    #[error("hedera submit error: {0}")]
    Submit(String),
}

/// Provider for interacting with a Hedera network as a facilitator.
#[derive(Clone)]
pub struct HederaChainProvider {
    chain: HederaChainReference,
    fee_payers: Vec<HederaFeePayer>,
    mirror: HederaMirrorClient,
    node_url: Option<String>,
}

impl Debug for HederaChainProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HederaChainProvider")
            .field("chain", &self.chain)
            .field(
                "fee_payers",
                &self
                    .fee_payers
                    .iter()
                    .map(|p| p.account_id.to_string())
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl HederaChainProvider {
    /// Creates a provider for `chain` using the given fee payers.
    ///
    /// Mirror REST defaults to the public Mirror Node for `chain`.
    #[must_use]
    pub fn new(chain: HederaChainReference, fee_payers: Vec<HederaFeePayer>) -> Self {
        Self {
            chain,
            fee_payers,
            mirror: HederaMirrorClient::connect(chain.default_mirror_url()),
            node_url: None,
        }
    }

    /// Overrides the Mirror Node REST base URL.
    #[must_use]
    pub fn with_mirror_url(mut self, url: impl Into<String>) -> Self {
        self.mirror = HederaMirrorClient::connect(url.into());
        self
    }

    /// Overrides the consensus node endpoint (`Client::for_network`).
    #[must_use]
    pub fn with_node_url(mut self, url: impl Into<String>) -> Self {
        self.node_url = Some(url.into());
        self
    }

    /// The Hedera network this provider is bound to.
    #[must_use]
    pub const fn chain_reference(&self) -> HederaChainReference {
        self.chain
    }

    /// Fee-payer account ids managed by this provider.
    #[must_use]
    pub fn fee_payer_ids(&self) -> Vec<String> {
        self.fee_payers
            .iter()
            .map(|p| p.account_id.to_string())
            .collect()
    }

    /// Shared Mirror Node handle.
    #[must_use]
    pub const fn mirror(&self) -> &HederaMirrorClient {
        &self.mirror
    }

    /// Verifies that `payer` signed the frozen transaction body.
    ///
    /// # Errors
    ///
    /// Returns [`HederaProviderError`] when Mirror Node lookup fails.
    pub async fn verify_payer_signature(
        &self,
        payer: &str,
        transaction_base64: &str,
        _network: &str,
    ) -> Result<HederaSignatureResult, HederaProviderError> {
        Ok(verify_payer_signature(&self.mirror, payer, transaction_base64).await?)
    }

    /// Preflight balance and token-association checks.
    ///
    /// # Errors
    ///
    /// Returns [`HederaProviderError`] when Mirror Node lookup fails.
    pub async fn preflight_transfer(
        &self,
        payer: &str,
        pay_to: &str,
        asset: &str,
        amount: &str,
        _network: &str,
    ) -> Result<HederaPreflightResult, HederaProviderError> {
        Ok(preflight_transfer(&self.mirror, payer, pay_to, asset, amount).await?)
    }

    /// Resolves `payTo` for alias policy.
    ///
    /// # Errors
    ///
    /// Returns [`HederaProviderError`] when Mirror Node lookup fails.
    pub async fn resolve_account(
        &self,
        account_id_or_alias: &str,
        _network: &str,
    ) -> Result<HederaAccountResolution, HederaProviderError> {
        Ok(resolve_account(&self.mirror, account_id_or_alias).await?)
    }

    /// Signs as `fee_payer` and submits, waiting for a `SUCCESS` receipt.
    ///
    /// # Errors
    ///
    /// Returns [`HederaProviderError`] when the fee payer is unknown, the
    /// payload is not a `TransferTransaction`, or execute/receipt fails.
    pub async fn sign_and_submit(
        &self,
        transaction_base64: &str,
        fee_payer: &str,
        network: &str,
    ) -> Result<String, HederaProviderError> {
        let payer = self
            .fee_payers
            .iter()
            .find(|p| p.account_id.to_string() == fee_payer)
            .ok_or_else(|| HederaProviderError::Parse(format!("unknown_fee_payer:{fee_payer}")))?;

        let bytes = Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            transaction_base64,
        )
        .map_err(|e| HederaProviderError::Parse(e.to_string()))?;
        let any = AnyTransaction::from_bytes(&bytes)
            .map_err(|e| HederaProviderError::Parse(e.to_string()))?;
        let mut tx = any
            .downcast::<TransferTransaction>()
            .map_err(|_| HederaProviderError::Parse("expected TransferTransaction".to_owned()))?;
        let _ = tx.sign(payer.private_key.clone());

        let client = create_hedera_client(network, self.node_url.as_deref())
            .map_err(HederaProviderError::Submit)?;
        let response = tx
            .execute(&client)
            .await
            .map_err(|e| HederaProviderError::Submit(e.to_string()))?;
        response
            .get_receipt(&client)
            .await
            .map_err(|e| HederaProviderError::Submit(e.to_string()))?;
        drop(client);
        Ok(response.transaction_id.to_string())
    }
}

impl ChainProvider for HederaChainProvider {
    fn signer_addresses(&self) -> Vec<String> {
        self.fee_payer_ids()
    }

    fn chain_id(&self) -> ChainId {
        self.chain.into()
    }
}
