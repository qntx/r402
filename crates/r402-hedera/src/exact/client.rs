//! Client-side payment signing for the Hedera `"exact"` scheme.

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;

use hedera::{AccountId, PrivateKey};
use r402_client::{DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use r402_protocol::error::ClientError;
use r402_protocol::scheme::SchemeId;
use r402_protocol::{Base64Bytes, ChainId, PaymentRequired, ResourceInfo};

use crate::chain::tx::create_partially_signed_transfer;
use crate::exact::{ExactHederaPayload, HederaExact, payload};

/// Local Hedera signer wrapping an account id and private key.
#[derive(Clone)]
pub struct HederaSigner {
    account_id: AccountId,
    private_key: PrivateKey,
    node_url: Option<String>,
}

impl std::fmt::Debug for HederaSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HederaSigner")
            .field("account_id", &self.account_id)
            .field("public_key", &self.private_key.public_key())
            .finish_non_exhaustive()
    }
}

impl HederaSigner {
    /// Constructs a signer from an account id and a hex-encoded private key.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Signing`] if the account id or key is invalid.
    pub fn from_secret(
        account_id: impl AsRef<str>,
        private_key: impl AsRef<str>,
    ) -> Result<Self, ClientError> {
        let account_id = AccountId::from_str(account_id.as_ref())
            .map_err(|e| ClientError::Signing(e.to_string()))?;
        let private_key = PrivateKey::from_str(private_key.as_ref())
            .map_err(|e| ClientError::Signing(e.to_string()))?;
        Ok(Self {
            account_id,
            private_key,
            node_url: None,
        })
    }

    /// Constructs a signer from already-parsed Hiero types.
    #[must_use]
    #[allow(
        clippy::large_types_passed_by_value,
        reason = "Hiero AccountId is Copy but includes optional key material"
    )]
    pub const fn new(account_id: AccountId, private_key: PrivateKey) -> Self {
        Self {
            account_id,
            private_key,
            node_url: None,
        }
    }

    /// Overrides the consensus node used to freeze the transaction.
    #[must_use]
    pub fn with_node_url(mut self, url: impl Into<String>) -> Self {
        self.node_url = Some(url.into());
        self
    }

    /// Account id this signer pays from.
    #[must_use]
    pub const fn account_id(&self) -> AccountId {
        self.account_id
    }
}

/// Builds a base64 payer-signed `TransferTransaction`.
///
/// # Errors
///
/// Returns [`ClientError`] if identifiers, amounts, or signing fail.
pub fn create_signed_transfer(
    signer: &HederaSigner,
    pay_to: &str,
    asset: &str,
    amount: &str,
    fee_payer: &str,
    network: &str,
) -> Result<String, ClientError> {
    let amount: i64 = amount
        .parse()
        .map_err(|e| ClientError::Signing(format!("{e}")))?;
    if amount <= 0 {
        return Err(ClientError::Signing(
            "amount must be greater than zero".to_owned(),
        ));
    }
    let pay_to = AccountId::from_str(pay_to).map_err(|e| ClientError::Signing(e.to_string()))?;
    let fee_payer =
        AccountId::from_str(fee_payer).map_err(|e| ClientError::Signing(e.to_string()))?;
    create_partially_signed_transfer(
        signer.account_id,
        &signer.private_key,
        fee_payer,
        pay_to,
        asset,
        amount,
        network,
        signer.node_url.as_deref(),
    )
    .map_err(ClientError::Signing)
}

/// Hedera exact scheme client for building and signing payment payloads.
#[derive(Clone)]
pub struct HederaExactClient<S> {
    signer: S,
}

impl<S> std::fmt::Debug for HederaExactClient<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HederaExactClient").finish_non_exhaustive()
    }
}

impl<S> HederaExactClient<S> {
    /// Creates a new Hedera exact client.
    pub const fn new(signer: S) -> Self {
        Self { signer }
    }
}

impl<S> SchemeId for HederaExactClient<S> {
    fn namespace(&self) -> &str {
        HederaExact.namespace()
    }

    fn scheme(&self) -> &str {
        HederaExact.scheme()
    }
}

impl<S> SchemeClient for HederaExactClient<S>
where
    S: AsRef<HederaSigner> + Send + Sync + Clone + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: payload::v2::PaymentRequirements = v.as_concrete()?;
                let chain_id = requirements.network.clone();
                if chain_id.namespace() != "hedera" {
                    return None;
                }
                requirements.extra.as_ref()?;
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

    fn find_default_asset(&self, asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
        crate::find_default_hedera_asset(asset, network)
    }
}

impl AsRef<Self> for HederaSigner {
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
    S: AsRef<HederaSigner> + Send + Sync,
{
    fn sign_payment(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + '_>> {
        Box::pin(async move {
            let extra = self.requirements.extra.as_ref().ok_or_else(|| {
                ClientError::Signing(
                    "feePayer is required in paymentRequirements.extra for Hedera exact".to_owned(),
                )
            })?;
            let b64 = create_signed_transfer(
                self.signer.as_ref(),
                self.requirements.pay_to.as_str(),
                self.requirements.asset.as_str(),
                self.requirements.amount.as_str(),
                extra.fee_payer.as_str(),
                &self.requirements.network.to_string(),
            )?;
            let payload = payload::v2::PaymentPayload::new(
                self.requirements.clone(),
                ExactHederaPayload { transaction: b64 },
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
    use hedera::PrivateKey;

    use super::*;

    #[test]
    fn signs_token_transfer() {
        let key = PrivateKey::generate_ed25519();
        let signer = HederaSigner::new(AccountId::from_str("0.0.9001").unwrap(), key);
        let b64 = create_signed_transfer(
            &signer,
            "0.0.7001",
            "0.0.6001",
            "1000",
            "0.0.5001",
            "hedera:testnet",
        )
        .unwrap();
        let inspected = crate::chain::inspect_hedera_transaction(&b64).unwrap();
        assert_eq!(inspected.transaction_id_account_id, "0.0.5001");
        assert!(!inspected.has_non_transfer_operations);
    }

    #[test]
    fn requires_positive_amount() {
        let key = PrivateKey::generate_ed25519();
        let signer = HederaSigner::new(AccountId::from_str("0.0.9001").unwrap(), key);
        let err = create_signed_transfer(
            &signer,
            "0.0.7001",
            "0.0.6001",
            "0",
            "0.0.5001",
            "hedera:testnet",
        )
        .unwrap_err();
        assert!(err.to_string().contains("greater than zero"));
    }
}
