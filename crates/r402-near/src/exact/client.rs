//! Client-side payment signing for the NEAR `"exact"` scheme.

use std::str::FromStr;

use near_crypto::{InMemorySigner, SecretKey, Signer};
use near_primitives::action::delegate::{DelegateAction, NonDelegateAction, SignedDelegateAction};
use near_primitives::action::{Action, FunctionCallAction};
use near_primitives::gas::Gas;
use near_primitives::types::{AccountId, Balance};
use r402_core::chain::ChainId;
use r402_core::error::ClientError;
use r402_core::scheme::SchemeId;
use r402_core::scheme::{DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use r402_core::wire::Base64Bytes;
use r402_core::wire::PaymentRequired;
use r402_core::wire::ResourceInfo;

use crate::chain::rpc::NearRpc;
use crate::exact::types;
use crate::exact::{ExactNearPayload, NearExact};
use crate::timeout_blocks;
use crate::{DEFAULT_FT_TRANSFER_GAS, FT_TRANSFER_METHOD, ONE_YOCTO};

/// Local NEAR signer wrapping an in-memory key.
#[derive(Clone)]
pub struct NearSigner {
    inner: Signer,
}

impl std::fmt::Debug for NearSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NearSigner")
            .field("account_id", &self.inner.get_account_id())
            .field("public_key", &self.inner.public_key())
            .finish_non_exhaustive()
    }
}

impl NearSigner {
    /// Constructs a signer from an account ID and a NEAR secret key string.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Signing`] if the account ID or secret key is invalid.
    pub fn from_secret_key(
        account_id: impl AsRef<str>,
        secret_key: impl AsRef<str>,
    ) -> Result<Self, ClientError> {
        let account_id = AccountId::from_str(account_id.as_ref())
            .map_err(|e| ClientError::Signing(e.to_string()))?;
        let secret_key = SecretKey::from_str(secret_key.as_ref())
            .map_err(|e| ClientError::Signing(e.to_string()))?;
        Ok(Self {
            inner: InMemorySigner::from_secret_key(account_id, secret_key),
        })
    }

    /// Account ID this signer pays from.
    #[must_use]
    pub fn account_id(&self) -> AccountId {
        self.inner.get_account_id()
    }

    /// Public key used to sign the delegate action.
    #[must_use]
    pub fn public_key(&self) -> near_crypto::PublicKey {
        self.inner.public_key()
    }
}

/// Builds a base64 Borsh `SignedDelegate` for one `ft_transfer`.
///
/// # Errors
///
/// Returns [`ClientError`] if RPC reads fail or the delegate cannot be signed.
pub async fn create_signed_delegate_action<R: NearRpc>(
    signer: &NearSigner,
    rpc: &R,
    pay_to: &str,
    asset: &str,
    amount: &str,
    max_timeout_seconds: u64,
) -> Result<String, ClientError> {
    let account_id = signer.account_id();
    let public_key = signer.public_key();
    let public_key_str = public_key.to_string();

    let access_key = rpc
        .view_access_key(account_id.as_str(), &public_key_str)
        .await
        .map_err(|e| ClientError::Signing(e.to_string()))?
        .ok_or_else(|| ClientError::Signing("access key not found".to_owned()))?;
    let height = rpc
        .current_block_height()
        .await
        .map_err(|e| ClientError::Signing(e.to_string()))?;

    let nonce = access_key.nonce.saturating_add(1);
    let max_block_height = height.saturating_add(timeout_blocks(max_timeout_seconds));

    let args = serde_json::to_vec(&serde_json::json!({
        "receiver_id": pay_to,
        "amount": amount,
    }))
    .map_err(|e| ClientError::Signing(e.to_string()))?;

    let function_call = FunctionCallAction {
        method_name: FT_TRANSFER_METHOD.to_owned(),
        args,
        gas: Gas::from_gas(DEFAULT_FT_TRANSFER_GAS),
        deposit: Balance::from_yoctonear(ONE_YOCTO),
    };
    let action = Action::FunctionCall(Box::new(function_call));
    let non_delegate =
        NonDelegateAction::try_from(action).map_err(|e| ClientError::Signing(e.to_string()))?;

    let receiver_id =
        AccountId::from_str(asset).map_err(|e| ClientError::Signing(e.to_string()))?;
    let delegate_action = DelegateAction {
        sender_id: account_id,
        receiver_id,
        actions: vec![non_delegate],
        nonce,
        max_block_height,
        public_key,
    };
    let signed = SignedDelegateAction::sign(&signer.inner, delegate_action);
    let bytes = borsh::to_vec(&signed).map_err(|e| ClientError::Signing(e.to_string()))?;
    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        bytes,
    ))
}

/// NEAR exact scheme client for building and signing payment payloads.
#[derive(Clone)]
pub struct NearExactClient<S, R> {
    signer: S,
    rpc: R,
}

impl<S, R> std::fmt::Debug for NearExactClient<S, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NearExactClient").finish_non_exhaustive()
    }
}

impl<S, R> NearExactClient<S, R> {
    /// Creates a new NEAR exact client.
    pub const fn new(signer: S, rpc: R) -> Self {
        Self { signer, rpc }
    }
}

impl<S, R> SchemeId for NearExactClient<S, R> {
    fn namespace(&self) -> &str {
        NearExact.namespace()
    }

    fn scheme(&self) -> &str {
        NearExact.scheme()
    }
}

impl<S, R> r402_core::scheme::Sealed for NearExactClient<S, R> {}

impl<S, R> SchemeClient for NearExactClient<S, R>
where
    S: AsRef<NearSigner> + Send + Sync + Clone + 'static,
    R: NearRpc + Clone + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: types::v2::PaymentRequirements = v.as_concrete()?;
                let chain_id = requirements.network.clone();
                if chain_id.namespace() != "near" {
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
                        rpc: self.rpc.clone(),
                        requirements,
                        resource: payment_required.resource.clone(),
                    }),
                })
            })
            .collect()
    }

    fn find_default_asset(&self, asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
        crate::find_default_near_asset(asset, network)
    }
}

impl AsRef<Self> for NearSigner {
    fn as_ref(&self) -> &Self {
        self
    }
}

struct V2PayloadSigner<S, R> {
    signer: S,
    rpc: R,
    requirements: types::v2::PaymentRequirements,
    resource: ResourceInfo,
}

impl<S, R> PaymentCandidateSigner for V2PayloadSigner<S, R>
where
    S: AsRef<NearSigner> + Send + Sync,
    R: NearRpc,
{
    fn sign_payment(&self) -> r402_core::facilitator::BoxFuture<'_, Result<String, ClientError>> {
        Box::pin(async move {
            let b64 = create_signed_delegate_action(
                self.signer.as_ref(),
                &self.rpc,
                self.requirements.pay_to.as_str(),
                self.requirements.asset.as_str(),
                self.requirements.amount.as_str(),
                self.requirements.max_timeout_seconds,
            )
            .await?;
            let payload = types::v2::PaymentPayload::new(
                self.requirements.clone(),
                ExactNearPayload {
                    signed_delegate_action: b64,
                },
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

    #[tokio::test]
    async fn live_sign_skipped_without_env() {
        if std::env::var("NEAR_ACCOUNT_ID").is_err() || std::env::var("NEAR_SECRET_KEY").is_err() {
            return;
        }
        let account = std::env::var("NEAR_ACCOUNT_ID").unwrap();
        let secret = std::env::var("NEAR_SECRET_KEY").unwrap();
        let signer = NearSigner::from_secret_key(&account, &secret).unwrap();
        let rpc = crate::chain::NearJsonRpc::connect(crate::chain::NEAR_TESTNET_RPC_URL);
        let result = create_signed_delegate_action(
            &signer,
            &rpc,
            "merchant.testnet",
            "usdc.testnet",
            "1",
            60,
        )
        .await;
        assert!(result.is_ok(), "live sign failed: {result:?}");
    }
}
