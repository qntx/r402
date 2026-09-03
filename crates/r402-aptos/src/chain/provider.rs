//! Facilitator-side Aptos provider: fee-payer keys + `aptos-sdk` 0.6.

use std::fmt::{Debug, Formatter};
use std::sync::Arc;

use aptos_sdk::account::Ed25519Account;
use aptos_sdk::crypto::Ed25519PrivateKey;
use aptos_sdk::transaction::PartiallySigned;
use aptos_sdk::transaction::types::FeePayerRawTransaction;
use aptos_sdk::types::AccountAddress;
use aptos_sdk::{Aptos, AptosConfig};
use r402_protocol::network::{ChainId, ChainProvider};

use super::account::AptosChainReference;
use super::codec::{AptosCodecError, DecodedAptosPayment, aptos_config_for, signed_from_decoded};

/// A fee-payer account this facilitator controls.
#[derive(Clone)]
pub struct AptosFeePayer {
    account: Ed25519Account,
}

impl Debug for AptosFeePayer {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AptosFeePayer")
            .field("address", &self.account.address().to_long_string())
            .finish_non_exhaustive()
    }
}

impl AptosFeePayer {
    /// Constructs a fee payer from a 32-byte Ed25519 private key hex.
    ///
    /// # Errors
    ///
    /// Returns [`AptosProviderError::Parse`] if the key is invalid.
    pub fn from_private_key_hex(private_key: impl AsRef<str>) -> Result<Self, AptosProviderError> {
        let account = Ed25519Account::from_private_key_hex(private_key.as_ref())
            .map_err(|e| AptosProviderError::Parse(e.to_string()))?;
        Ok(Self { account })
    }

    /// Constructs a fee payer from an SDK Ed25519 account.
    #[must_use]
    pub const fn new(account: Ed25519Account) -> Self {
        Self { account }
    }

    /// Constructs a fee payer from raw private-key bytes.
    ///
    /// # Errors
    ///
    /// Returns [`AptosProviderError::Parse`] if the bytes are not a valid key.
    pub fn from_private_key_bytes(bytes: &[u8]) -> Result<Self, AptosProviderError> {
        let key = Ed25519PrivateKey::from_bytes(bytes)
            .map_err(|e| AptosProviderError::Parse(e.to_string()))?;
        Ok(Self {
            account: Ed25519Account::from_private_key(key),
        })
    }

    /// Fee-payer long-form address.
    #[must_use]
    pub fn address_long(&self) -> String {
        self.account.address().to_long_string()
    }

    /// SDK account.
    #[must_use]
    pub const fn account(&self) -> &Ed25519Account {
        &self.account
    }
}

/// Errors from the Aptos facilitator provider.
#[derive(Debug, thiserror::Error)]
pub enum AptosProviderError {
    /// Identifier or key could not be parsed.
    #[error("aptos provider parse error: {0}")]
    Parse(String),
    /// Fullnode / view / simulate request failed.
    #[error("aptos rpc error: {0}")]
    Rpc(String),
    /// Settlement submit failed.
    #[error("aptos submit error: {0}")]
    Submit(String),
    /// Payload could not be decoded.
    #[error(transparent)]
    Codec(#[from] AptosCodecError),
}

/// Result of a simulated payment transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AptosSimulationResult {
    /// Whether the VM would succeed.
    pub success: bool,
    /// VM status string.
    pub vm_status: String,
}

/// Provider for interacting with an Aptos network as a facilitator.
#[derive(Clone)]
pub struct AptosChainProvider {
    chain: AptosChainReference,
    fee_payers: Vec<AptosFeePayer>,
    aptos: Arc<Aptos>,
    sponsor_transactions: bool,
}

impl Debug for AptosChainProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AptosChainProvider")
            .field("chain", &self.chain)
            .field(
                "fee_payers",
                &self
                    .fee_payers
                    .iter()
                    .map(AptosFeePayer::address_long)
                    .collect::<Vec<_>>(),
            )
            .field("sponsor_transactions", &self.sponsor_transactions)
            .finish_non_exhaustive()
    }
}

impl AptosChainProvider {
    /// Creates a provider for `chain` using the given fee payers and optional RPC URL.
    ///
    /// # Errors
    ///
    /// Returns [`AptosProviderError`] if the SDK client cannot be constructed.
    pub fn new(
        chain: AptosChainReference,
        fee_payers: Vec<AptosFeePayer>,
        rpc_url: Option<&str>,
    ) -> Result<Self, AptosProviderError> {
        let config = aptos_config_for(chain.chain_id(), rpc_url)
            .map_err(|e| AptosProviderError::Parse(e.to_string()))?;
        Self::from_config(chain, fee_payers, config)
    }

    /// Creates a provider from an explicit [`AptosConfig`].
    ///
    /// # Errors
    ///
    /// Returns [`AptosProviderError::Rpc`] if the HTTP client fails to build.
    pub fn from_config(
        chain: AptosChainReference,
        fee_payers: Vec<AptosFeePayer>,
        config: AptosConfig,
    ) -> Result<Self, AptosProviderError> {
        let aptos = Aptos::new(config).map_err(|e| AptosProviderError::Rpc(e.to_string()))?;
        Ok(Self {
            chain,
            fee_payers,
            aptos: Arc::new(aptos),
            sponsor_transactions: true,
        })
    }

    /// Disables or enables fee-payer sponsorship advertised on `/supported`.
    #[must_use]
    pub const fn with_sponsor_transactions(mut self, sponsor: bool) -> Self {
        self.sponsor_transactions = sponsor;
        self
    }

    /// Whether this facilitator advertises `extra.feePayer`.
    #[must_use]
    pub const fn sponsor_transactions(&self) -> bool {
        self.sponsor_transactions
    }

    /// Fee-payer long-form addresses this provider will sign for.
    #[must_use]
    pub fn fee_payer_addresses(&self) -> Vec<String> {
        self.fee_payers
            .iter()
            .map(AptosFeePayer::address_long)
            .collect()
    }

    /// The Aptos network this provider is bound to.
    #[must_use]
    pub const fn chain_reference(&self) -> AptosChainReference {
        self.chain
    }

    /// Shared SDK client.
    #[must_use]
    pub fn aptos(&self) -> &Aptos {
        self.aptos.as_ref()
    }

    /// Looks up a managed fee payer by address.
    #[must_use]
    pub fn fee_payer_for(&self, address: AccountAddress) -> Option<&AptosFeePayer> {
        self.fee_payers
            .iter()
            .find(|p| p.account.address() == address)
    }

    /// Current fungible-asset balance of `owner` for `asset` (FA metadata).
    ///
    /// # Errors
    ///
    /// Returns [`AptosProviderError`] if addresses or the view call fail.
    pub async fn fungible_asset_balance(
        &self,
        owner: &str,
        asset: &str,
    ) -> Result<u64, AptosProviderError> {
        let owner = AccountAddress::from_hex(owner)
            .map_err(|e| AptosProviderError::Parse(e.to_string()))?;
        let asset = AccountAddress::from_hex(asset)
            .map_err(|e| AptosProviderError::Parse(e.to_string()))?;
        let values = self
            .aptos
            .view(
                "0x1::primary_fungible_store::balance",
                vec!["0x1::fungible_asset::Metadata".to_owned()],
                vec![
                    serde_json::Value::String(owner.to_long_string()),
                    serde_json::Value::String(asset.to_long_string()),
                ],
            )
            .await
            .map_err(|e| AptosProviderError::Rpc(e.to_string()))?;
        parse_u64_view(&values)
    }

    /// Simulates the decoded payment via `aptos-sdk` 0.6.
    ///
    /// # Errors
    ///
    /// Returns [`AptosProviderError`] if decode or simulate fails.
    pub async fn simulate_payment(
        &self,
        transaction_base64: &str,
    ) -> Result<AptosSimulationResult, AptosProviderError> {
        let decoded = DecodedAptosPayment::from_base64(transaction_base64)?;
        let signed = signed_from_decoded(&decoded);
        let result = self
            .aptos
            .simulate_signed(&signed)
            .await
            .map_err(|e| AptosProviderError::Rpc(e.to_string()))?;
        Ok(AptosSimulationResult {
            success: result.success(),
            vm_status: result.vm_status().to_owned(),
        })
    }

    /// Adds the fee-payer signature when sponsored and submits via the SDK.
    ///
    /// # Errors
    ///
    /// Returns [`AptosProviderError`] if decode, signing, or submit fails.
    pub async fn sign_and_submit(
        &self,
        transaction_base64: &str,
        fee_payer: Option<&str>,
    ) -> Result<String, AptosProviderError> {
        let decoded = DecodedAptosPayment::from_base64(transaction_base64)?;
        let signed = if let Some(fee_payer) = fee_payer {
            let fee_payer_addr = AccountAddress::from_hex(fee_payer)
                .map_err(|e| AptosProviderError::Parse(e.to_string()))?;
            let payer = self.fee_payer_for(fee_payer_addr).ok_or_else(|| {
                AptosProviderError::Submit(format!(
                    "fee payer {} is not managed by this facilitator",
                    fee_payer_addr.to_long_string()
                ))
            })?;
            let mut partial = PartiallySigned::new(FeePayerRawTransaction::new_simple(
                decoded.transaction.raw_transaction.clone(),
                fee_payer_addr,
            ));
            partial.sender_auth = Some(decoded.sender_authenticator.clone());
            partial
                .sign_as_fee_payer(payer.account())
                .map_err(|e| AptosProviderError::Submit(e.to_string()))?;
            partial
                .finalize()
                .map_err(|e| AptosProviderError::Submit(e.to_string()))?
        } else {
            signed_from_decoded(&decoded)
        };

        let result = self
            .aptos
            .submit_and_wait(&signed, None)
            .await
            .map_err(|e| AptosProviderError::Submit(e.to_string()))?;
        if let Some(hash) = result.data.get("hash").and_then(serde_json::Value::as_str) {
            return Ok(hash.to_owned());
        }
        signed
            .hash()
            .map(|h| h.to_string())
            .map_err(|e| AptosProviderError::Submit(e.to_string()))
    }
}

impl ChainProvider for AptosChainProvider {
    fn signer_addresses(&self) -> Vec<String> {
        self.fee_payer_addresses()
    }

    fn chain_id(&self) -> ChainId {
        self.chain.into()
    }
}

fn parse_u64_view(values: &[serde_json::Value]) -> Result<u64, AptosProviderError> {
    let first = values.first().ok_or_else(|| {
        AptosProviderError::Rpc("empty view response for fungible asset balance".to_owned())
    })?;
    if let Some(n) = first.as_u64() {
        return Ok(n);
    }
    if let Some(s) = first.as_str() {
        return s
            .parse()
            .map_err(|e| AptosProviderError::Rpc(format!("invalid balance view: {e}")));
    }
    Err(AptosProviderError::Rpc(format!(
        "unexpected balance view: {first}"
    )))
}
