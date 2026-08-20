//! Client-side payment signing for the Keeta `"exact"` scheme.

use std::str::FromStr;
use std::sync::Arc;

use keetanetwork_account::{
    Account, AccountPublicKey, Accountable, GenericAccount, KeyED25519, KeyPairType, Keyable,
};
use keetanetwork_block::{AccountRef, Amount};
use keetanetwork_client::UserClient;
use keetanetwork_crypto::prelude::IntoSecret;
use r402_core::error::ClientError;
use r402_core::scheme::SchemeId;
use r402_core::scheme::{PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use r402_core::wire::Base64Bytes;
use r402_core::wire::PaymentRequired;
use r402_core::wire::ResourceInfo;

use crate::chain::KeetaChainReference;
use crate::chain::types::account_has_private_key;
use crate::exact::types;
use crate::exact::{ExactKeetaPayload, KeetaExact};

/// Local Keeta signer wrapping a keyed account.
#[derive(Clone)]
pub struct KeetaSigner {
    account: AccountRef,
}

impl std::fmt::Debug for KeetaSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeetaSigner")
            .field("address", &self.account.to_string())
            .finish_non_exhaustive()
    }
}

impl KeetaSigner {
    /// Wraps an account that must hold a private key.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Signing`] when the account cannot sign.
    pub fn from_account(account: AccountRef) -> Result<Self, ClientError> {
        if !account_has_private_key(&account) {
            return Err(ClientError::Signing(format!(
                "account {account} has no private key"
            )));
        }
        Ok(Self { account })
    }

    /// Derives an Ed25519 signer from a 32-byte seed and derivation index.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Signing`] when key derivation fails.
    pub fn from_ed25519_seed(seed: [u8; 32], index: u32) -> Result<Self, ClientError> {
        let account = Account::<KeyED25519>::try_from(Accountable::KeyAndType(
            Keyable::Seed((seed.into_secret(), index)),
            KeyPairType::ED25519,
        ))
        .map_err(|e| ClientError::Signing(e.to_string()))?;
        Self::from_account(Arc::new(GenericAccount::Ed25519(account)))
    }

    /// Address this signer pays from.
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

struct DestroyOnDrop(UserClient);

impl Drop for DestroyOnDrop {
    fn drop(&mut self) {
        self.0.client().destroy();
    }
}

/// Builds a base64 DER signed `SEND` block.
///
/// # Errors
///
/// Returns [`ClientError`] if the client cannot be bound or the block cannot
/// be built and signed.
pub async fn create_signed_block(
    signer: &KeetaSigner,
    chain: KeetaChainReference,
    pay_to: &str,
    asset: &str,
    amount: &str,
    external: Option<&str>,
) -> Result<String, ClientError> {
    let pay_to = parse_account(pay_to)?;
    let token = parse_account(asset)?;
    if token.to_keypair_type() != KeyPairType::TOKEN {
        return Err(ClientError::Signing(format!(
            "asset {asset} is not a token account"
        )));
    }
    let amount: Amount = amount
        .parse()
        .map_err(|e: <Amount as FromStr>::Err| ClientError::Signing(e.to_string()))?;

    let writer =
        UserClient::from_network(chain.client_network(), Some(Arc::clone(&signer.account)))
            .map_err(|e| ClientError::Signing(e.to_string()))?;
    let writer = DestroyOnDrop(writer);
    let mut builder = writer
        .0
        .init_builder()
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    match external {
        Some(ext) => {
            builder.send_external(&pay_to, &token, amount, ext);
        }
        None => {
            builder.send(&pay_to, &token, amount);
        }
    }
    let blocks = builder
        .build()
        .await
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let payment = blocks
        .first()
        .ok_or_else(|| ClientError::Signing("payment block not found".to_owned()))?;
    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        payment.to_bytes(),
    ))
}

fn parse_account(s: &str) -> Result<AccountRef, ClientError> {
    let account = GenericAccount::from_str(s).map_err(|e| ClientError::Signing(e.to_string()))?;
    Ok(Arc::new(account))
}

/// Keeta exact scheme client for building and signing payment payloads.
#[derive(Clone)]
pub struct KeetaExactClient<S> {
    signer: S,
}

impl<S> std::fmt::Debug for KeetaExactClient<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeetaExactClient").finish_non_exhaustive()
    }
}

impl<S> KeetaExactClient<S> {
    /// Creates a new Keeta exact client.
    pub const fn new(signer: S) -> Self {
        Self { signer }
    }
}

impl<S> SchemeId for KeetaExactClient<S> {
    fn namespace(&self) -> &str {
        KeetaExact.namespace()
    }

    fn scheme(&self) -> &str {
        KeetaExact.scheme()
    }
}

impl<S> r402_core::scheme::Sealed for KeetaExactClient<S> {}

impl<S> SchemeClient for KeetaExactClient<S>
where
    S: AsRef<KeetaSigner> + Send + Sync + Clone + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: types::v2::PaymentRequirements = v.as_concrete()?;
                let chain_id = requirements.network.clone();
                if chain_id.namespace() != "keeta" {
                    return None;
                }
                let chain = KeetaChainReference::try_from(chain_id.clone()).ok()?;
                Some(PaymentCandidate {
                    chain_id,
                    asset: requirements.asset.to_string().into(),
                    amount: requirements.amount.as_str().into(),
                    scheme: self.scheme().into(),
                    pay_to: requirements.pay_to.to_string().into(),
                    signer: Box::new(V2PayloadSigner {
                        signer: self.signer.clone(),
                        chain,
                        requirements,
                        resource: payment_required.resource.clone(),
                    }),
                })
            })
            .collect()
    }
}

impl AsRef<Self> for KeetaSigner {
    fn as_ref(&self) -> &Self {
        self
    }
}

struct V2PayloadSigner<S> {
    signer: S,
    chain: KeetaChainReference,
    requirements: types::v2::PaymentRequirements,
    resource: ResourceInfo,
}

impl<S> PaymentCandidateSigner for V2PayloadSigner<S>
where
    S: AsRef<KeetaSigner> + Send + Sync,
{
    fn sign_payment(&self) -> r402_core::facilitator::BoxFuture<'_, Result<String, ClientError>> {
        Box::pin(async move {
            let external = self
                .requirements
                .extra
                .as_ref()
                .and_then(|extra| extra.external.as_deref());
            let b64 = create_signed_block(
                self.signer.as_ref(),
                self.chain,
                self.requirements.pay_to.as_str(),
                self.requirements.asset.as_str(),
                self.requirements.amount.as_str(),
                external,
            )
            .await?;
            let payload = types::v2::PaymentPayload::new(
                self.requirements.clone(),
                ExactKeetaPayload { block: b64 },
            )
            .with_resource(self.resource.clone());
            let json = serde_json::to_vec(&payload)?;
            let encoded = Base64Bytes::encode(&json);
            Ok(encoded.to_string())
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test assertions"
)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn live_sign_skipped_without_env() {
        if std::env::var("KEETA_SEED_HEX").is_err() {
            return;
        }
        let hex = std::env::var("KEETA_SEED_HEX").unwrap();
        let seed = parse_seed_hex(hex.trim()).expect("KEETA_SEED_HEX must be 64 hex chars");
        let signer = KeetaSigner::from_ed25519_seed(seed, 0).unwrap();
        let result = create_signed_block(
            &signer,
            KeetaChainReference::TESTNET,
            signer.address().as_str(),
            crate::USDC::keeta_testnet().address.as_str(),
            "1",
            None,
        )
        .await;
        assert!(result.is_ok(), "live sign failed: {result:?}");
    }

    fn parse_seed_hex(s: &str) -> Option<[u8; 32]> {
        if s.len() != 64 {
            return None;
        }
        let mut seed = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks_exact(2).enumerate() {
            let hi = from_hex(chunk[0])?;
            let lo = from_hex(chunk[1])?;
            seed[i] = (hi << 4) | lo;
        }
        Some(seed)
    }

    const fn from_hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
}
