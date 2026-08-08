//! Type definitions for the Casper "exact" payment scheme.
//!
//! The wire shapes mirror the Casper x402 reference SDKs
//! (<https://github.com/make-software/casper-x402>): the buyer signs an
//! EIP-712 `TransferWithAuthorization` message over the CEP-18 token's
//! domain, and the facilitator relays it to the token's
//! `transfer_with_authorization` entry point. Wire format type aliases live
//! in the [`v2`] sub-module.

pub use r402_core::scheme::ExactScheme;
use serde::{Deserialize, Serialize};

use crate::chain::{Address, PublicKey};
use crate::exact::CasperExactError;
use crate::hex;
use crate::motes::Motes;

/// Length of an EIP-712 `TransferWithAuthorization` signature in bytes.
///
/// Casper reuses the Ethereum layout: 64 bytes of `r ‖ s` plus one recovery
/// / algorithm byte.
pub const SIGNATURE_LEN: usize = 65;

/// Length of an authorisation nonce in bytes.
pub const NONCE_LEN: usize = 32;

/// Minimum remaining validity, in seconds, required at verification time.
///
/// A payment whose `validBefore` is closer than this cannot realistically be
/// included in a block before it expires, so the facilitator rejects it up
/// front instead of burning gas on a doomed transaction. Aligned with the
/// Casper reference SDKs.
pub const MIN_SETTLEMENT_WINDOW_SECS: u64 = 6;

/// Signed authorisation fields for a Casper exact payment.
///
/// Field names match the EIP-712 struct the buyer signs, so the JSON is
/// byte-compatible with the TypeScript and Go Casper x402 SDKs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExactCasperAuthorization {
    /// Payer addressable key (`00`-tagged account hash).
    pub from: Address,
    /// Recipient addressable key (`00`-tagged account hash).
    pub to: Address,
    /// Amount in the token's base units (motes for wCSPR).
    pub value: Motes,
    /// Unix timestamp (seconds) after which the authorisation is valid.
    #[serde(with = "unix_seconds")]
    pub valid_after: u64,
    /// Unix timestamp (seconds) before which the authorisation must be used.
    #[serde(with = "unix_seconds")]
    pub valid_before: u64,
    /// 32-byte replay-protection nonce, hex encoded.
    pub nonce: String,
}

impl ExactCasperAuthorization {
    /// Decodes the nonce into its 32 raw bytes.
    ///
    /// # Errors
    ///
    /// Returns [`CasperExactError::InvalidNonce`] when the nonce is not
    /// exactly 32 bytes of hex.
    pub fn nonce_bytes(&self) -> Result<[u8; NONCE_LEN], CasperExactError> {
        hex::decode_exact::<NONCE_LEN>(&self.nonce)
            .map_err(|e| CasperExactError::InvalidNonce(e.to_string()))
    }

    /// Returns the validity window in seconds, or `None` when the bounds are
    /// inverted.
    #[must_use]
    pub const fn validity_window(&self) -> Option<u64> {
        self.valid_before.checked_sub(self.valid_after)
    }
}

/// Payload of a Casper exact payment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExactCasperPayload {
    /// 65-byte EIP-712 signature, hex encoded.
    pub signature: String,
    /// Full tagged public key of the payer.
    pub public_key: PublicKey,
    /// The signed authorisation fields.
    pub authorization: ExactCasperAuthorization,
}

impl ExactCasperPayload {
    /// Decodes the signature into its raw bytes.
    ///
    /// # Errors
    ///
    /// Returns [`CasperExactError::InvalidSignature`] when the signature is
    /// not exactly [`SIGNATURE_LEN`] bytes of hex.
    pub fn signature_bytes(&self) -> Result<[u8; SIGNATURE_LEN], CasperExactError> {
        hex::decode_exact::<SIGNATURE_LEN>(&self.signature)
            .map_err(|e| CasperExactError::InvalidSignature(e.to_string()))
    }
}

/// Scheme-specific `extra` block on Casper payment requirements.
///
/// The CEP-18 contract binds its EIP-712 domain to `name` and `version`, so
/// both are mandatory: without them the buyer cannot compute the digest the
/// contract will check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CasperPaymentRequirementsExtra {
    /// EIP-712 domain `name` of the CEP-18 token contract.
    pub name: compact_str::CompactString,
    /// EIP-712 domain `version` of the CEP-18 token contract.
    pub version: compact_str::CompactString,
    /// Optional token symbol, for display purposes only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<compact_str::CompactString>,
    /// Optional token decimals, stringified for wire parity with the
    /// reference SDKs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<compact_str::CompactString>,
}

impl CasperPaymentRequirementsExtra {
    /// Builds the mandatory half of the extra block.
    pub fn new(
        name: impl Into<compact_str::CompactString>,
        version: impl Into<compact_str::CompactString>,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            symbol: None,
            decimals: None,
        }
    }

    /// Builder: attaches the display symbol.
    #[must_use]
    pub fn with_symbol(mut self, symbol: impl Into<compact_str::CompactString>) -> Self {
        self.symbol = Some(symbol.into());
        self
    }

    /// Builder: attaches the token decimals.
    #[must_use]
    pub fn with_decimals(mut self, decimals: u8) -> Self {
        self.decimals = Some(compact_str::CompactString::from(decimals.to_string()));
        self
    }
}

/// Scheme-specific `extra` block advertised on `/supported` payment kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportedPaymentKindExtra {
    /// Facilitator account that submits (and pays gas for) settlement.
    pub fee_payer: Address,
}

/// Serialises `u64` timestamps as decimal strings, matching the reference
/// SDKs (JavaScript loses precision on large JSON integers).
mod unix_seconds {
    use serde::{Deserialize, Deserializer, Serializer};

    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "signature is dictated by serde's `serialize_with` contract"
    )]
    pub(super) fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.trim().parse::<u64>().map_err(serde::de::Error::custom)
    }
}

/// Wire format type aliases for the Casper exact scheme.
///
/// Uses CAIP-2 chain IDs (`casper:casper`, `casper:casper-test`) for chain
/// identification and embeds the accepted requirements directly in the
/// payload, per x402 v2.
pub mod v2 {
    use r402_core::wire as proto_v2;

    use super::{
        CasperPaymentRequirementsExtra, ExactCasperPayload, ExactScheme, Motes,
        SupportedPaymentKindExtra,
    };
    use crate::chain::Address;

    /// Type alias for verify requests using the exact Casper payment scheme.
    pub type VerifyRequest = proto_v2::TypedVerifyRequest<2, PaymentPayload, PaymentRequirements>;

    /// Type alias for settle requests (same structure as verify requests).
    pub type SettleRequest = VerifyRequest;

    /// Type alias for payment payloads with embedded requirements and
    /// Casper-specific data.
    pub type PaymentPayload = proto_v2::PaymentPayload<PaymentRequirements, ExactCasperPayload>;

    /// Type alias for payment requirements with Casper-specific types.
    ///
    /// Note that `asset` is a **contract package hash** while `payTo` is a
    /// tagged addressable key; both are represented as strings on the wire,
    /// so the shared `TAddress` parameter stays the wire-level string and the
    /// scheme parses each field into its own type.
    pub type PaymentRequirements = proto_v2::PaymentRequirements<
        ExactScheme,
        Motes,
        compact_str::CompactString,
        CasperPaymentRequirementsExtra,
    >;

    /// Strongly-typed view over the recipient address carried in
    /// [`PaymentRequirements::pay_to`].
    pub type PayTo = Address;

    /// Re-export so downstream code can name the supported-kind extras
    /// without reaching into the parent module.
    pub type SupportedExtra = SupportedPaymentKindExtra;
}

#[allow(
    clippy::indexing_slicing,
    reason = "tests index serde_json values; panic-on-missing-key is the desired assertion behaviour"
)]
#[cfg(test)]
mod tests {
    use super::*;

    /// Secp256k1 fixture from `scheme_exact_casper.md` (publicKey → from).
    const ACCOUNT: &str = "0076d080b4e769f0b29c77fc6472d6e425710840c2f46a4506e5544d2ce34f43a3";
    const PUBLIC_KEY: &str = "020376e4f8766e4f33bcc6e20b331b5163f363dc0106063b052ad38afe08637bd867";
    const PAYEE: &str = "00fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321";

    fn payload_json() -> serde_json::Value {
        serde_json::json!({
            "signature": format!("02{}", "aa".repeat(64)),
            "publicKey": PUBLIC_KEY,
            "authorization": {
                "from": ACCOUNT,
                "to": PAYEE,
                "value": "1500000000",
                "validAfter": "1700000000",
                "validBefore": "1700000600",
                "nonce": "cc".repeat(32),
            }
        })
    }

    #[test]
    fn payload_round_trips_reference_sdk_json() {
        let json = payload_json();
        let payload: ExactCasperPayload = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(payload.authorization.value, Motes::new(1_500_000_000));
        assert_eq!(payload.authorization.valid_after, 1_700_000_000);
        assert_eq!(payload.authorization.valid_before, 1_700_000_600);
        assert_eq!(serde_json::to_value(&payload).unwrap(), json);
    }

    #[test]
    fn payload_rejects_unknown_fields() {
        let mut json = payload_json();
        json["extraField"] = serde_json::json!(1);
        assert!(serde_json::from_value::<ExactCasperPayload>(json).is_err());
    }

    #[test]
    fn signature_and_nonce_decode_to_fixed_widths() {
        let payload: ExactCasperPayload = serde_json::from_value(payload_json()).unwrap();
        assert_eq!(payload.signature_bytes().unwrap().len(), SIGNATURE_LEN);
        assert_eq!(
            payload.authorization.nonce_bytes().unwrap().len(),
            NONCE_LEN
        );
    }

    #[test]
    fn short_signature_is_rejected() {
        let mut json = payload_json();
        json["signature"] = serde_json::json!("aa".repeat(64));
        let payload: ExactCasperPayload = serde_json::from_value(json).unwrap();
        assert!(matches!(
            payload.signature_bytes().unwrap_err(),
            CasperExactError::InvalidSignature(_)
        ));
    }

    #[test]
    fn short_nonce_is_rejected() {
        let mut json = payload_json();
        json["authorization"]["nonce"] = serde_json::json!("cc".repeat(16));
        let payload: ExactCasperPayload = serde_json::from_value(json).unwrap();
        assert!(matches!(
            payload.authorization.nonce_bytes().unwrap_err(),
            CasperExactError::InvalidNonce(_)
        ));
    }

    #[test]
    fn timestamps_are_stringified_on_the_wire() {
        let payload: ExactCasperPayload = serde_json::from_value(payload_json()).unwrap();
        let json = serde_json::to_value(&payload).unwrap();
        assert!(
            json["authorization"]["validAfter"].is_string(),
            "timestamps must be strings for cross-SDK compatibility"
        );
        assert_eq!(payload.authorization.validity_window(), Some(600));
    }

    #[test]
    fn requirements_extra_serialises_camel_case() {
        let extra = CasperPaymentRequirementsExtra::new("Wrapped CSPR", "1")
            .with_symbol("WCSPR")
            .with_decimals(9);
        let json = serde_json::to_value(&extra).unwrap();
        assert_eq!(json["name"], "Wrapped CSPR");
        assert_eq!(json["version"], "1");
        assert_eq!(json["symbol"], "WCSPR");
        assert_eq!(json["decimals"], "9");
    }

    #[test]
    fn requirements_extra_omits_optional_fields() {
        let extra = CasperPaymentRequirementsExtra::new("Wrapped CSPR", "1");
        let json = serde_json::to_value(&extra).unwrap();
        assert!(json.get("symbol").is_none());
        assert!(json.get("decimals").is_none());
    }

    #[test]
    fn supported_extra_round_trips() {
        let extra = SupportedPaymentKindExtra {
            fee_payer: ACCOUNT.parse().unwrap(),
        };
        let json = serde_json::to_value(extra).unwrap();
        assert_eq!(json["feePayer"], ACCOUNT);
        assert_eq!(
            serde_json::from_value::<SupportedPaymentKindExtra>(json).unwrap(),
            extra
        );
    }

    #[test]
    fn typed_verify_request_decodes_v2_envelope() {
        let request = serde_json::json!({
            "x402Version": 2,
            "paymentPayload": {
                "x402Version": 2,
                "accepted": {
                    "scheme": "exact",
                    "network": "casper:casper-test",
                    "amount": "1500000000",
                    "payTo": PAYEE,
                    "maxTimeoutSeconds": 300,
                    "asset": "ab".repeat(32),
                    "extra": { "name": "Wrapped CSPR", "version": "1" }
                },
                "payload": payload_json(),
            },
            "paymentRequirements": {
                "scheme": "exact",
                "network": "casper:casper-test",
                "amount": "1500000000",
                "payTo": PAYEE,
                "maxTimeoutSeconds": 300,
                "asset": "ab".repeat(32),
                "extra": { "name": "Wrapped CSPR", "version": "1" }
            }
        });
        let typed =
            v2::VerifyRequest::from_verify(r402_core::wire::VerifyRequest::from(request)).unwrap();
        assert_eq!(typed.payment_requirements.amount, Motes::new(1_500_000_000));
        assert_eq!(
            typed.payment_requirements.network.to_string(),
            "casper:casper-test"
        );
        assert_eq!(
            typed.payment_payload.accepted.extra.unwrap().name,
            "Wrapped CSPR"
        );
    }
}
