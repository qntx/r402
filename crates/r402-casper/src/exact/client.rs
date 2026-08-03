//! Client-side payload assembly for the Casper exact scheme.
//!
//! The buyer signs an EIP-712 `TransferWithAuthorization` message over the
//! CEP-18 token's domain with their Casper key; this module builds the
//! authorisation that gets signed and assembles the resulting x402 v2
//! payment payload. Key custody stays with the caller — r402 never handles
//! Casper private keys — so the signature is supplied by the wallet or
//! signer the application already uses.

use r402_core::wire;

use crate::chain::{Address, ContractPackageHash, PublicKey};
use crate::exact::types::v2;
use crate::exact::{
    CasperExactError, ExactCasperAuthorization, ExactCasperPayload, NONCE_LEN,
    validate_payload_shape,
};
use crate::hex;
use crate::motes::Motes;

/// The EIP-712 primary type the buyer signs.
pub const PRIMARY_TYPE: &str = "TransferWithAuthorization";

/// Field names and Solidity-style types of [`PRIMARY_TYPE`], in signing
/// order. Exposed so callers can drive an EIP-712 hasher without
/// re-declaring the layout.
pub const TRANSFER_WITH_AUTHORIZATION_FIELDS: &[(&str, &str)] = &[
    ("from", "address"),
    ("to", "address"),
    ("value", "uint256"),
    ("validAfter", "uint256"),
    ("validBefore", "uint256"),
    ("nonce", "bytes32"),
];

/// The EIP-712 domain a Casper CEP-18 contract binds its authorisations to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eip712Domain {
    /// Token contract name, from `requirements.extra.name`.
    pub name: String,
    /// Token contract version, from `requirements.extra.version`.
    pub version: String,
    /// CAIP-2 network identifier the payment targets.
    pub network: String,
    /// CEP-18 contract package hash acting as the verifying contract.
    pub verifying_contract: ContractPackageHash,
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
    ///
    /// The validity window defaults to `[now, now + max_timeout_seconds]`
    /// taken from the requirements when built via
    /// [`Self::for_requirements`].
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
            .valid_from(now)
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

/// Assembles a signed payment payload.
///
/// `signature` is the 65-byte EIP-712 signature produced by the buyer's
/// Casper key, hex encoded.
///
/// # Errors
///
/// Returns [`CasperExactError`] when the signature, nonce, or addresses are
/// malformed — the payload is validated before it can be sent.
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
    wire::PaymentPayload::new(accepted, payload)
}

/// Derives the EIP-712 domain for a set of requirements.
///
/// # Errors
///
/// Returns [`CasperExactError`] when `asset` is not a package hash or when
/// the `extra` block is missing the token name/version the domain requires.
pub fn domain_for(requirements: &v2::PaymentRequirements) -> Result<Eip712Domain, CasperExactError> {
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
    Ok(Eip712Domain {
        name: extra.name.to_string(),
        version: extra.version.to_string(),
        network: requirements.network.to_string(),
        verifying_contract,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYER: &str = "001234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
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
        format!("01{}", "bb".repeat(32)).parse().unwrap()
    }

    #[test]
    fn builder_derives_window_from_requirements() {
        let auth =
            AuthorizationBuilder::for_requirements(PAYER.parse().unwrap(), &requirements(), 1_000)
                .unwrap()
                .nonce([7u8; NONCE_LEN])
                .build();
        assert_eq!(auth.valid_after, 1_000);
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
        let auth = AuthorizationBuilder::for_requirements(
            PAYER.parse().unwrap(),
            &requirements(),
            1_000,
        )
        .unwrap()
        .build();
        let payload = build_payload(auth.clone(), public_key(), "aa".repeat(65)).unwrap();
        assert_eq!(payload.authorization, auth);

        let err = build_payload(auth, public_key(), "aa".repeat(10)).unwrap_err();
        assert!(matches!(err, CasperExactError::InvalidSignature(_)));
    }

    #[test]
    fn payment_payload_envelope_is_x402_v2() {
        let auth = AuthorizationBuilder::for_requirements(
            PAYER.parse().unwrap(),
            &requirements(),
            1_000,
        )
        .unwrap()
        .build();
        let payload = build_payload(auth, public_key(), "aa".repeat(65)).unwrap();
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
        assert_eq!(PRIMARY_TYPE, "TransferWithAuthorization");
        let names: Vec<&str> = TRANSFER_WITH_AUTHORIZATION_FIELDS
            .iter()
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(
            names,
            vec!["from", "to", "value", "validAfter", "validBefore", "nonce"]
        );
    }
}
