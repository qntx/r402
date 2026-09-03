//! Client-side payment signing for the Aptos `"exact"` scheme.

use std::future::Future;
use std::pin::Pin;

use aptos_sdk::Aptos;
use aptos_sdk::account::Ed25519Account;
use aptos_sdk::crypto::Ed25519PrivateKey;
use aptos_sdk::transaction::InputEntryFunctionData;
use aptos_sdk::transaction::builder::TransactionBuilder;
use aptos_sdk::types::{AccountAddress, ChainId};
use r402_client::{DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use r402_protocol::scheme::SchemeId;
use r402_protocol::{Base64Bytes, ClientError, PaymentRequired, ResourceInfo};

use crate::chain::codec::{
    SimpleTransaction, account_authenticator_for, aptos_config_for, encode_aptos_payload,
};
use crate::chain::{
    AptosChainReference, DEFAULT_CLIENT_MAX_GAS, DEFAULT_GAS_UNIT_PRICE, MAX_GAS_UNIT_PRICE,
};
use crate::exact::{AptosExact, ExactAptosPayload, payload};

/// Local Aptos signer wrapping an Ed25519 account.
#[derive(Clone)]
pub struct AptosSigner {
    account: Ed25519Account,
    rpc_url: Option<String>,
}

impl std::fmt::Debug for AptosSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AptosSigner")
            .field("address", &self.account.address().to_long_string())
            .finish_non_exhaustive()
    }
}

impl AptosSigner {
    /// Constructs a signer from a 32-byte Ed25519 private key hex.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Signing`] if the key is invalid.
    pub fn from_private_key_hex(private_key: impl AsRef<str>) -> Result<Self, ClientError> {
        let account = Ed25519Account::from_private_key_hex(private_key.as_ref())
            .map_err(|e| ClientError::Signing(e.to_string()))?;
        Ok(Self {
            account,
            rpc_url: None,
        })
    }

    /// Constructs a signer from already-parsed SDK types.
    #[must_use]
    pub const fn new(account: Ed25519Account) -> Self {
        Self {
            account,
            rpc_url: None,
        }
    }

    /// Constructs a signer from raw private-key bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Signing`] if the bytes are not a valid key.
    pub fn from_private_key_bytes(bytes: &[u8]) -> Result<Self, ClientError> {
        let key = Ed25519PrivateKey::from_bytes(bytes)
            .map_err(|e| ClientError::Signing(e.to_string()))?;
        Ok(Self::new(Ed25519Account::from_private_key(key)))
    }

    /// Overrides the fullnode URL used to fetch sequence number and gas.
    #[must_use]
    pub fn with_rpc_url(mut self, url: impl Into<String>) -> Self {
        self.rpc_url = Some(url.into());
        self
    }

    /// Long-form address this signer pays from.
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

/// Builds a TS-encoded payload from already-resolved chain parameters.
///
/// # Errors
///
/// Returns [`ClientError`] if identifiers, amounts, or signing fail.
#[allow(
    clippy::too_many_arguments,
    reason = "explicit tx fields keep the SDK builder call local"
)]
pub fn create_signed_transfer(
    signer: &AptosSigner,
    pay_to: &str,
    asset: &str,
    amount: &str,
    fee_payer: Option<&str>,
    chain_id: u8,
    sequence_number: u64,
    max_gas_amount: u64,
    gas_unit_price: u64,
    expiration_timestamp_secs: u64,
) -> Result<String, ClientError> {
    let amount: u64 = amount
        .parse()
        .map_err(|e| ClientError::Signing(format!("{e}")))?;
    if amount == 0 {
        return Err(ClientError::Signing(
            "amount must be greater than zero".to_owned(),
        ));
    }
    let pay_to =
        AccountAddress::from_hex(pay_to).map_err(|e| ClientError::Signing(e.to_string()))?;
    let asset = AccountAddress::from_hex(asset).map_err(|e| ClientError::Signing(e.to_string()))?;
    let fee_payer = fee_payer
        .map(AccountAddress::from_hex)
        .transpose()
        .map_err(|e| ClientError::Signing(e.to_string()))?;

    let payload = InputEntryFunctionData::transfer_fungible_asset(asset, pay_to, amount)
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let raw = TransactionBuilder::new()
        .sender(signer.account.address())
        .sequence_number(sequence_number)
        .payload(payload)
        .max_gas_amount(max_gas_amount)
        .gas_unit_price(gas_unit_price)
        .expiration_timestamp_secs(expiration_timestamp_secs)
        .chain_id(ChainId::new(chain_id))
        .build()
        .map_err(|e| ClientError::Signing(e.to_string()))?;

    let transaction = SimpleTransaction {
        raw_transaction: raw,
        fee_payer_address: fee_payer,
    };
    let message = transaction
        .signing_message()
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let authenticator = account_authenticator_for(&signer.account, &message)
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    encode_aptos_payload(&transaction, &authenticator)
        .map_err(|e| ClientError::Signing(e.to_string()))
}

/// Builds a signed transfer after reading sequence number and gas from the node.
///
/// # Errors
///
/// Returns [`ClientError`] if RPC reads fail or signing fails.
pub async fn create_signed_transfer_from_network(
    signer: &AptosSigner,
    pay_to: &str,
    asset: &str,
    amount: &str,
    fee_payer: Option<&str>,
    network: &str,
    max_timeout_seconds: u64,
) -> Result<String, ClientError> {
    let chain = parse_chain_from_caip(network)?;

    let config = aptos_config_for(chain.chain_id(), signer.rpc_url.as_deref())
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let aptos = Aptos::new(config).map_err(|e| ClientError::Signing(e.to_string()))?;
    let sequence_number = aptos
        .get_sequence_number(signer.account.address())
        .await
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let gas_unit_price = match aptos.fullnode().estimate_gas_price().await {
        Ok(est) => est.data.recommended().min(MAX_GAS_UNIT_PRICE),
        Err(_) => DEFAULT_GAS_UNIT_PRICE,
    };
    let expiration = now_secs().saturating_add(max_timeout_seconds.max(1));
    create_signed_transfer(
        signer,
        pay_to,
        asset,
        amount,
        fee_payer,
        chain.chain_id(),
        sequence_number,
        DEFAULT_CLIENT_MAX_GAS,
        gas_unit_price,
        expiration,
    )
}

fn parse_chain_from_caip(network: &str) -> Result<AptosChainReference, ClientError> {
    let chain_id: r402_protocol::ChainId = network
        .parse()
        .map_err(|e: r402_protocol::ChainIdFormatError| ClientError::Signing(e.to_string()))?;
    AptosChainReference::try_from(chain_id).map_err(|e| ClientError::Signing(e.to_string()))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Aptos exact scheme client for building and signing payment payloads.
#[derive(Clone)]
pub struct AptosExactClient<S> {
    signer: S,
}

impl<S> std::fmt::Debug for AptosExactClient<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AptosExactClient").finish_non_exhaustive()
    }
}

impl<S> AptosExactClient<S> {
    /// Creates a new Aptos exact client.
    pub const fn new(signer: S) -> Self {
        Self { signer }
    }
}

impl<S> SchemeId for AptosExactClient<S> {
    fn namespace(&self) -> &str {
        AptosExact.namespace()
    }

    fn scheme(&self) -> &str {
        AptosExact.scheme()
    }
}

impl<S> SchemeClient for AptosExactClient<S>
where
    S: AsRef<AptosSigner> + Send + Sync + Clone + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: payload::v2::PaymentRequirements = v.as_concrete()?;
                let chain_id = requirements.network.clone();
                if chain_id.namespace() != "aptos" {
                    return None;
                }
                Some(PaymentCandidate {
                    chain_id,
                    asset: requirements.asset.to_string().into(),
                    amount: requirements.amount.as_str().into(),
                    scheme: self.scheme().into(),
                    pay_to: requirements.pay_to.to_string().into(),
                    requirements: v.clone(),
                    signer: Box::new(V2PayloadSigner {
                        signer: self.signer.clone(),
                        requirements,
                        resource: payment_required.resource.clone(),
                    }),
                })
            })
            .collect()
    }

    fn find_default_asset(
        &self,
        asset: &str,
        network: &r402_protocol::ChainId,
    ) -> Option<DefaultAssetInfo> {
        crate::find_default_aptos_asset(asset, network)
    }
}

impl AsRef<Self> for AptosSigner {
    fn as_ref(&self) -> &Self {
        self
    }
}

struct V2PayloadSigner<S> {
    signer: S,
    requirements: payload::v2::PaymentRequirements,
    resource: ResourceInfo,
}

impl<S> PaymentCandidateSigner for V2PayloadSigner<S>
where
    S: AsRef<AptosSigner> + Send + Sync,
{
    fn sign_payment(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + '_>> {
        Box::pin(async move {
            let fee_payer = self
                .requirements
                .extra
                .as_ref()
                .and_then(|extra| extra.fee_payer.as_ref())
                .map(ToString::to_string);
            let b64 = create_signed_transfer_from_network(
                self.signer.as_ref(),
                self.requirements.pay_to.as_str(),
                self.requirements.asset.as_str(),
                self.requirements.amount.as_str(),
                fee_payer.as_deref(),
                &self.requirements.network.to_string(),
                self.requirements.max_timeout_seconds,
            )
            .await?;
            let payload = payload::v2::PaymentPayload::new(
                self.requirements.clone(),
                ExactAptosPayload { transaction: b64 },
            )
            .with_resource(self.resource.clone());
            let json = serde_json::to_vec(&payload)?;
            let encoded = Base64Bytes::encode(&json);
            Ok(encoded.to_string())
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;
    use crate::chain::codec::DecodedAptosPayment;
    use crate::chain::{USDC_TESTNET_FA, is_supported_transfer};

    #[test]
    fn signs_sponsored_transfer() {
        let signer = AptosSigner::new(Ed25519Account::generate());
        let fee_payer = Ed25519Account::generate().address().to_long_string();
        let pay_to = "0x0000000000000000000000000000000000000000000000000000000000000001";
        let b64 = create_signed_transfer(
            &signer,
            pay_to,
            USDC_TESTNET_FA,
            "1000",
            Some(&fee_payer),
            2,
            0,
            DEFAULT_CLIENT_MAX_GAS,
            100,
            1_900_000_000,
        )
        .unwrap();
        let decoded = DecodedAptosPayment::from_base64(&b64).unwrap();
        assert_eq!(
            decoded
                .transaction
                .fee_payer_address
                .unwrap()
                .to_long_string(),
            fee_payer
        );
        assert!(is_supported_transfer(decoded.entry_function().unwrap()));
        decoded
            .sender_authenticator
            .verify(&decoded.transaction.signing_message().unwrap())
            .unwrap();
    }

    #[test]
    fn requires_positive_amount() {
        let signer = AptosSigner::new(Ed25519Account::generate());
        let err = create_signed_transfer(
            &signer,
            "0x0000000000000000000000000000000000000000000000000000000000000001",
            USDC_TESTNET_FA,
            "0",
            None,
            2,
            0,
            DEFAULT_CLIENT_MAX_GAS,
            100,
            1_900_000_000,
        )
        .unwrap_err();
        assert!(err.to_string().contains("greater than zero"));
    }
}
