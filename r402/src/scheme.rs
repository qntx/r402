//! Payment scheme protocol definitions.
//!
//! Defines the traits that payment scheme implementations must satisfy to
//! integrate with the x402 client, resource server, and facilitator roles.
//!
//! Corresponds to Python SDK's `interfaces.py`.

use r402_proto::{
    PaymentPayload, PaymentPayloadV1, PaymentRequirements, PaymentRequirementsV1, SettleResponse,
    SupportedKind, VerifyResponse,
};
use serde_json::Value;

/// V2 client-side payment mechanism.
///
/// Implementations create signed payment payloads for a specific scheme.
/// Returns the inner payload dict, which the x402 client wraps into a full
/// [`PaymentPayload`].
///
/// Corresponds to Python SDK's `SchemeNetworkClient`.
pub trait SchemeClient: Send + Sync {
    /// Payment scheme identifier (e.g., `"exact"`).
    fn scheme(&self) -> &str;

    /// Creates the scheme-specific inner payload.
    ///
    /// # Errors
    ///
    /// Returns an error if payload creation fails (e.g., signing error).
    fn create_payment_payload(
        &self,
        requirements: &PaymentRequirements,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>>;
}

/// V1 (legacy) client-side payment mechanism.
///
/// Same as [`SchemeClient`] but operates on V1 protocol types.
///
/// Corresponds to Python SDK's `SchemeNetworkClientV1`.
pub trait SchemeClientV1: Send + Sync {
    /// Payment scheme identifier.
    fn scheme(&self) -> &str;

    /// Creates the scheme-specific inner payload for V1.
    ///
    /// # Errors
    ///
    /// Returns an error if payload creation fails.
    fn create_payment_payload(
        &self,
        requirements: &PaymentRequirementsV1,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>>;
}

/// V2 server-side payment mechanism.
///
/// Implementations handle price parsing and requirement enhancement for a
/// specific scheme. Does **not** verify/settle — that is delegated to the
/// facilitator client.
///
/// Corresponds to Python SDK's `SchemeNetworkServer`.
pub trait SchemeServer: Send + Sync {
    /// Payment scheme identifier (e.g., `"exact"`).
    fn scheme(&self) -> &str;

    /// Converts a human-friendly price to an atomic asset amount.
    ///
    /// For example, converts `"1.50"` USD to `"1500000"` for USDC (6 decimals).
    ///
    /// # Errors
    ///
    /// Returns an error if the price format is invalid or the network is
    /// unsupported.
    fn parse_price(
        &self,
        price: &Value,
        network: &str,
    ) -> Result<AssetAmount, Box<dyn std::error::Error + Send + Sync>>;

    /// Adds scheme-specific fields to payment requirements.
    ///
    /// For EVM, this adds EIP-712 domain parameters (`name`, `version`) to
    /// the `extra` field.
    fn enhance_payment_requirements(
        &self,
        requirements: PaymentRequirements,
        supported_kind: &SupportedKind,
        extensions: &[String],
    ) -> PaymentRequirements;
}

/// V2 facilitator-side payment mechanism.
///
/// Implementations verify and settle payments for a specific scheme.
/// Returns response objects with `is_valid=false` / `success=false` on
/// failure, rather than raising exceptions.
///
/// Corresponds to Python SDK's `SchemeNetworkFacilitator`.
pub trait SchemeFacilitator: Send + Sync {
    /// Payment scheme identifier (e.g., `"exact"`).
    fn scheme(&self) -> &str;

    /// CAIP family pattern (e.g., `"eip155:*"` for EVM, `"solana:*"` for SVM).
    fn caip_family(&self) -> &str;

    /// Returns extra data for [`SupportedKind`].
    fn get_extra(&self, network: &str) -> Option<Value>;

    /// Returns signer addresses for a given network.
    fn get_signers(&self, network: &str) -> Vec<String>;

    /// Verifies a payment.
    fn verify(
        &self,
        payload: &PaymentPayload,
        requirements: &PaymentRequirements,
    ) -> VerifyResponse;

    /// Settles a payment on-chain.
    fn settle(
        &self,
        payload: &PaymentPayload,
        requirements: &PaymentRequirements,
    ) -> SettleResponse;
}

/// V1 (legacy) facilitator-side payment mechanism.
///
/// Corresponds to Python SDK's `SchemeNetworkFacilitatorV1`.
pub trait SchemeFacilitatorV1: Send + Sync {
    /// Payment scheme identifier.
    fn scheme(&self) -> &str;

    /// CAIP family pattern.
    fn caip_family(&self) -> &str;

    /// Returns extra data for [`SupportedKind`].
    fn get_extra(&self, network: &str) -> Option<Value>;

    /// Returns signer addresses for a given network.
    fn get_signers(&self, network: &str) -> Vec<String>;

    /// Verifies a V1 payment.
    fn verify(
        &self,
        payload: &PaymentPayloadV1,
        requirements: &PaymentRequirementsV1,
    ) -> VerifyResponse;

    /// Settles a V1 payment on-chain.
    fn settle(
        &self,
        payload: &PaymentPayloadV1,
        requirements: &PaymentRequirementsV1,
    ) -> SettleResponse;
}

/// Amount in smallest unit with asset identifier.
///
/// Corresponds to Python SDK's `AssetAmount` in `schemas/base.py`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetAmount {
    /// Amount in smallest unit (e.g., `"1500000"` for 1.5 USDC).
    pub amount: String,

    /// Asset address/identifier.
    pub asset: String,

    /// Optional additional metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}
