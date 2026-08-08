//! Client-side payload assembly and `SchemeClient` for the Casper exact scheme.
//!
//! The buyer signs a CEP-3009 EIP-712 `TransferWithAuthorization` digest with
//! their Casper key. Key custody stays with the caller via [`CasperSigner`];
//! this module builds the digest, authorisation, and x402 v2 envelope.

use std::future::Future;
use std::sync::Arc;

use r402_core::error::ClientError;
use r402_core::scheme::{
    PaymentCandidate, PaymentCandidateSigner, SchemeClient, SchemeId, sealed::Sealed,
};
use r402_core::wire::{Base64Bytes, PaymentRequired, ResourceInfo};
use rand::RngExt as _;
use rand::rng;

use crate::chain::{Address, ContractPackageHash, PublicKey};
use crate::exact::eip712::{
    Eip712Domain, domain_from_parts, transfer_with_authorization_digest,
};
use crate::exact::types::v2;
use crate::exact::{
    CasperExact, CasperExactError, ExactCasperAuthorization, ExactCasperPayload, NONCE_LEN,
    validate_payload_shape,
};
use crate::hex;
use crate::motes::Motes;

/// Field names and Solidity-style types of
/// [`PRIMARY_TYPE`](crate::exact::PRIMARY_TYPE), in signing order.
pub const TRANSFER_WITH_AUTHORIZATION_FIELDS: &[(&str, &str)] = &[
    ("from", "address"),
    ("to", "address"),
    ("value", "uint256"),
    ("validAfter", "uint256"),
    ("validBefore", "uint256"),
    ("nonce", "bytes32"),
];

/// Abstraction over Casper key material used for CEP-3009 signing.
///
/// Implement this for ed25519 / secp256k1 wallets. The returned signature
/// MUST be the full 65-byte wire form: algorithm tag (`0x01` / `0x02`)
/// followed by 64 signature bytes (matching `publicKey`'s tag).
pub trait CasperSigner: Send + Sync {
    /// Tagged public key of the payer.
    fn public_key(&self) -> PublicKey;

    /// Signs the 32-byte EIP-712 digest, returning a 65-byte tagged signature.
    fn sign_digest(
        &self,
        digest: &[u8; 32],
    ) -> impl Future<Output = Result<[u8; 65], ClientError>> + Send;
}

impl<T: CasperSigner + Send + Sync> CasperSigner for Arc<T> {
    fn public_key(&self) -> PublicKey {
        (**self).public_key()
    }

    async fn sign_digest(&self, digest: &[u8; 32]) -> Result<[u8; 65], ClientError> {
        (**self).sign_digest(digest).await
    }
}

/// Builder for a Casper exact payment authorisation.
#[derive(Debug, Clone, Copy)]
pub struct AuthorizationBuilder {
    from: Address,
    to: Address,
    value: Motes,
    valid_after: u64,
    valid_before: u64,
    nonce: [u8; NONCE_LEN],
}

impl AuthorizationBuilder {
    /// Starts an authorisation for `value` motes from `from` to `to`.
    #[must_use]
    pub const fn new(from: Address, to: Address, value: Motes) -> Self {
        Self {
            from,
            to,
            value,
            valid_after: 0,
            valid_before: 0,
            nonce: [0u8; NONCE_LEN],
        }
    }

    /// Derives a builder from the requirements the buyer is accepting.
    ///
    /// Uses `validAfter = now - 600` and `validBefore = now + maxTimeout`
    /// (aligned with the JS `ExactCasperScheme` client).
    ///
    /// # Errors
    ///
    /// Returns [`CasperExactError::InvalidPayTo`] when `requirements.pay_to`
    /// is not a valid Casper addressable key.
    pub fn for_requirements(
        from: Address,
        requirements: &v2::PaymentRequirements,
        now: u64,
    ) -> Result<Self, CasperExactError> {
        let to = requirements
            .pay_to
            .parse::<Address>()
            .map_err(|_| CasperExactError::InvalidPayTo(requirements.pay_to.to_string()))?;
        Ok(Self::new(from, to, requirements.amount)
            .valid_from(now.saturating_sub(600))
            .valid_for(now, requirements.max_timeout_seconds))
    }

    /// Sets the `validAfter` timestamp.
    #[must_use]
    pub const fn valid_from(mut self, valid_after: u64) -> Self {
        self.valid_after = valid_after;
        self
    }

    /// Sets `validBefore` to `now + seconds`, saturating on overflow.
    #[must_use]
    pub const fn valid_for(mut self, now: u64, seconds: u64) -> Self {
        self.valid_before = now.saturating_add(seconds);
        self
    }

    /// Sets an explicit `validBefore` timestamp.
    #[must_use]
    pub const fn valid_until(mut self, valid_before: u64) -> Self {
        self.valid_before = valid_before;
        self
    }

    /// Sets the 32-byte replay-protection nonce.
    #[must_use]
    pub const fn nonce(mut self, nonce: [u8; NONCE_LEN]) -> Self {
        self.nonce = nonce;
        self
    }

    /// Produces the authorisation the buyer must sign.
    #[must_use]
    pub fn build(self) -> ExactCasperAuthorization {
        ExactCasperAuthorization {
            from: self.from,
            to: self.to,
            value: self.value,
            valid_after: self.valid_after,
            valid_before: self.valid_before,
            nonce: hex::encode(&self.nonce),
        }
    }
}

/// Assembles a signed payment payload and validates its shape.
///
/// # Errors
///
/// Returns [`CasperExactError`] when the signature, nonce, or addresses are
/// malformed.
pub fn build_payload(
    authorization: ExactCasperAuthorization,
    public_key: PublicKey,
    signature: impl Into<String>,
) -> Result<ExactCasperPayload, CasperExactError> {
    let payload = ExactCasperPayload {
        signature: signature.into(),
        public_key,
        authorization,
    };
    validate_payload_shape(&payload)?;
    Ok(payload)
}

/// Wraps a payload into the x402 v2 `PaymentPayload` envelope.
#[must_use]
pub fn build_payment_payload(
    accepted: v2::PaymentRequirements,
    payload: ExactCasperPayload,
) -> v2::PaymentPayload {
    r402_core::wire::PaymentPayload::new(accepted, payload)
}

/// Derives the EIP-712 domain for a set of requirements.
///
/// # Errors
///
/// Returns [`CasperExactError`] when `asset` or domain `extra` fields are invalid.
pub fn domain_for(
    requirements: &v2::PaymentRequirements,
) -> Result<Eip712Domain, CasperExactError> {
    let verifying_contract = requirements
        .asset
        .parse::<ContractPackageHash>()
        .map_err(|_| CasperExactError::InvalidAsset(requirements.asset.to_string()))?;
    let extra = requirements
        .extra
        .as_ref()
        .ok_or(CasperExactError::MissingTokenName)?;
    if extra.name.trim().is_empty() {
        return Err(CasperExactError::MissingTokenName);
    }
    if extra.version.trim().is_empty() {
        return Err(CasperExactError::MissingTokenVersion);
    }
    Ok(domain_from_parts(
        extra.name.as_str(),
        extra.version.as_str(),
        requirements.network.to_string(),
        verifying_contract,
    ))
}

/// Casper exact-scheme client that plugs into `r402-http`'s `X402Client`.
#[derive(Clone)]
pub struct CasperExactClient<S> {
    signer: S,
}

impl<S: std::fmt::Debug> std::fmt::Debug for CasperExactClient<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CasperExactClient")
            .field("signer", &self.signer)
            .finish()
    }
}

impl<S> CasperExactClient<S> {
    /// Creates a client with the given Casper signer.
    pub const fn new(signer: S) -> Self {
        Self { signer }
    }
}

impl<S> SchemeId for CasperExactClient<S> {
    fn namespace(&self) -> &str {
        CasperExact.namespace()
    }

    fn scheme(&self) -> &str {
        CasperExact.scheme()
    }
}

impl<S> Sealed for CasperExactClient<S> {}

impl<S> SchemeClient for CasperExactClient<S>
where
    S: CasperSigner + Clone + Send + Sync + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: v2::PaymentRequirements = v.as_concrete()?;
                // Reject non-casper networks early.
                let _ = crate::chain::CasperChainReference::try_from(requirements.network.clone())
                    .ok()?;
                Some(PaymentCandidate {
                    chain_id: requirements.network.clone(),
                    asset: requirements.asset.clone(),
                    amount: requirements.amount.to_string().into(),
                    scheme: self.scheme().into(),
                    pay_to: requirements.pay_to.clone(),
                    signer: Box::new(V2PayloadSigner {
                        resource_info: Some(payment_required.resource.clone()),
                        signer: self.signer.clone(),
                        requirements,
                    }),
                })
            })
            .collect()
    }
}

struct V2PayloadSigner<S> {
    signer: S,
    resource_info: Option<ResourceInfo>,
    requirements: v2::PaymentRequirements,
}

impl<S> PaymentCandidateSigner for V2PayloadSigner<S>
where
    S: CasperSigner + Sync,
{
    fn sign_payment(&self) -> r402_core::facilitator::BoxFuture<'_, Result<String, ClientError>> {
        Box::pin(async move {
            let public_key = self.signer.public_key();
            let from = public_key.account_hash();
            let now = crate::exact::verify::now_unix();
            let authorization = AuthorizationBuilder::for_requirements(
                from,
                &self.requirements,
                now,
            )
            .map_err(|e| ClientError::Signing(e.to_string()))?
            .nonce(rng().random())
            .build();

            let domain =
                domain_for(&self.requirements).map_err(|e| ClientError::Signing(e.to_string()))?;
            let digest = transfer_with_authorization_digest(&domain, &authorization);
            let signature = self.signer.sign_digest(&digest).await?;
            let signature_hex = hex::encode(&signature);

            let payload = build_payload(authorization, public_key, signature_hex)
                .map_err(|e| ClientError::Signing(e.to_string()))?;
            let envelope = build_payment_payload(self.requirements.clone(), payload)
                .with_optional_resource(self.resource_info.clone());
            let json = serde_json::to_vec(&envelope)?;
            Ok(Base64Bytes::encode(&json).to_string())
        })
    }
}

#[allow(
    clippy::indexing_slicing,
    reason = "tests index serde_json values; panic-on-missing-key is the desired assertion behaviour"
)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::exact::eip712::{PRIMARY_TYPE as EIP712_PRIMARY, TRANSFER_WITH_AUTHORIZATION_TYPE};

    /// Secp256k1 fixture from `scheme_exact_casper.md` (publicKey → from).
    const PAYER: &str = "0076d080b4e769f0b29c77fc6472d6e425710840c2f46a4506e5544d2ce34f43a3";
    const PUBLIC_KEY: &str = "020376e4f8766e4f33bcc6e20b331b5163f363dc0106063b052ad38afe08637bd867";
    const PAYEE: &str = "00fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321";
    const ASSET: &str = "3d80df21ba4ee4d66a2a1f60c32570dd5685e4b279f6538162a5fd1314847c1e";

    fn requirements() -> v2::PaymentRequirements {
        serde_json::from_value(serde_json::json!({
            "scheme": "exact",
            "network": "casper:casper-test",
            "amount": "1500000000",
            "payTo": PAYEE,
            "maxTimeoutSeconds": 300,
            "asset": ASSET,
            "extra": { "name": "Wrapped CSPR", "version": "1" }
        }))
        .unwrap()
    }

    fn public_key() -> PublicKey {
        PUBLIC_KEY.parse().unwrap()
    }

    fn signature_hex() -> String {
        format!("02{}", "aa".repeat(64))
    }

    /// Stub signer that returns a fixed signature for plumbing tests.
    #[derive(Clone, Debug)]
    struct StubSigner;

    impl CasperSigner for StubSigner {
        fn public_key(&self) -> PublicKey {
            public_key()
        }

        fn sign_digest(
            &self,
            _digest: &[u8; 32],
        ) -> impl Future<Output = Result<[u8; 65], ClientError>> {
            let mut sig = [0u8; 65];
            sig[0] = 0x02;
            sig[1..].fill(0xaa);
            std::future::ready(Ok(sig))
        }
    }

    #[test]
    fn builder_derives_window_from_requirements() {
        let auth =
            AuthorizationBuilder::for_requirements(PAYER.parse().unwrap(), &requirements(), 1_000)
                .unwrap()
                .nonce([7u8; NONCE_LEN])
                .build();
        assert_eq!(auth.valid_after, 400);
        assert_eq!(auth.valid_before, 1_300);
        assert_eq!(auth.value, Motes::new(1_500_000_000));
        assert_eq!(auth.to.to_string(), PAYEE);
        assert_eq!(auth.nonce, "07".repeat(NONCE_LEN));
    }

    #[test]
    fn builder_saturates_instead_of_overflowing() {
        let auth = AuthorizationBuilder::new(
            PAYER.parse().unwrap(),
            PAYEE.parse().unwrap(),
            Motes::new(1),
        )
        .valid_for(u64::MAX, 300)
        .build();
        assert_eq!(auth.valid_before, u64::MAX);
    }

    #[test]
    fn build_payload_validates_signature_width() {
        let auth =
            AuthorizationBuilder::for_requirements(PAYER.parse().unwrap(), &requirements(), 1_000)
                .unwrap()
                .build();
        let payload = build_payload(auth.clone(), public_key(), signature_hex()).unwrap();
        assert_eq!(payload.authorization, auth);

        let err = build_payload(auth, public_key(), "aa".repeat(10)).unwrap_err();
        assert!(matches!(err, CasperExactError::InvalidSignature(_)));
    }

    #[test]
    fn payment_payload_envelope_is_x402_v2() {
        let auth =
            AuthorizationBuilder::for_requirements(PAYER.parse().unwrap(), &requirements(), 1_000)
                .unwrap()
                .build();
        let payload = build_payload(auth, public_key(), signature_hex()).unwrap();
        let envelope = build_payment_payload(requirements(), payload);
        let json = serde_json::to_value(&envelope).unwrap();
        assert_eq!(json["x402Version"], 2);
        assert_eq!(json["accepted"]["network"], "casper:casper-test");
        assert_eq!(json["payload"]["authorization"]["value"], "1500000000");
    }

    #[test]
    fn domain_is_derived_from_asset_and_extra() {
        let domain = domain_for(&requirements()).unwrap();
        assert_eq!(domain.name, "Wrapped CSPR");
        assert_eq!(domain.version, "1");
        assert_eq!(domain.network, "casper:casper-test");
        assert_eq!(domain.verifying_contract.to_string(), ASSET);
    }

    #[test]
    fn domain_requires_token_name_and_version() {
        let mut requirements = requirements();
        requirements.extra = None;
        assert_eq!(
            domain_for(&requirements).unwrap_err(),
            CasperExactError::MissingTokenName
        );
    }

    #[test]
    fn primary_type_field_layout_matches_the_reference_sdks() {
        assert_eq!(EIP712_PRIMARY, "TransferWithAuthorization");
        assert!(TRANSFER_WITH_AUTHORIZATION_TYPE.contains("validAfter"));
        let names: Vec<&str> = TRANSFER_WITH_AUTHORIZATION_FIELDS
            .iter()
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(
            names,
            vec!["from", "to", "value", "validAfter", "validBefore", "nonce"]
        );
    }

    fn payment_required_json() -> PaymentRequired {
        serde_json::from_value(serde_json::json!({
            "x402Version": 2,
            "resource": { "url": "https://example.com/paid" },
            "accepts": [serde_json::to_value(requirements()).unwrap()]
        }))
        .unwrap()
    }

    #[test]
    fn scheme_client_accepts_casper_requirements() {
        let client = CasperExactClient::new(StubSigner);
        let candidates = client.accept(&payment_required_json());
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].scheme, "exact");
        assert_eq!(candidates[0].chain_id.to_string(), "casper:casper-test");
    }

    #[tokio::test]
    async fn scheme_client_signs_to_base64_payload() {
        let client = CasperExactClient::new(StubSigner);
        let candidates = client.accept(&payment_required_json());
        let b64 = candidates[0].sign().await.unwrap();
        assert!(!b64.is_empty());
        let raw = Base64Bytes(b64.into_bytes()).decode().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(json["x402Version"], 2);
        assert_eq!(json["payload"]["publicKey"], PUBLIC_KEY);
    }
}
