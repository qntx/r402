//! V1 legacy payment types for the x402 protocol.
//!
//! These types correspond to the original (V1) protocol version using
//! network name strings and a flat payload structure.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Network;

/// V1 payment requirements (legacy).
///
/// Uses `maxAmountRequired` instead of V2's `amount`, and includes resource
/// information inline rather than in a separate `ResourceInfo` struct.
///
/// Corresponds to Python SDK's `PaymentRequirementsV1` in `schemas/v1.py`.
///
/// # JSON Format
///
/// ```json
/// {
///   "scheme": "exact",
///   "network": "base-sepolia",
///   "maxAmountRequired": "1000000",
///   "resource": "/api/data",
///   "payTo": "0x...",
///   "maxTimeoutSeconds": 300,
///   "asset": "0x..."
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequirementsV1 {
    /// Payment scheme identifier (e.g., "exact").
    pub scheme: String,

    /// Network identifier (legacy format, e.g., "base-sepolia").
    pub network: Network,

    /// Maximum amount in smallest unit.
    pub max_amount_required: String,

    /// Resource URL.
    pub resource: String,

    /// Optional resource description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Optional MIME type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,

    /// Recipient address.
    pub pay_to: String,

    /// Maximum time in seconds for payment validity.
    pub max_timeout_seconds: u64,

    /// Asset address/identifier.
    pub asset: String,

    /// Optional output schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,

    /// Additional scheme-specific data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

impl PaymentRequirementsV1 {
    /// Returns the payment amount (V1 uses `maxAmountRequired`).
    #[must_use]
    pub fn amount(&self) -> &str {
        &self.max_amount_required
    }

    /// Returns the extra metadata.
    #[must_use]
    pub const fn extra(&self) -> Option<&Value> {
        self.extra.as_ref()
    }
}

/// V1 402 response (legacy).
///
/// Corresponds to Python SDK's `PaymentRequiredV1` in `schemas/v1.py`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequiredV1 {
    /// Protocol version (always 1 for V1).
    #[serde(default = "default_v1")]
    pub x402_version: u32,

    /// Optional error message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// List of accepted payment requirements.
    pub accepts: Vec<PaymentRequirementsV1>,
}

/// V1 payment payload (legacy).
///
/// In V1, `scheme` and `network` are at the top level rather than nested
/// inside an `accepted` field.
///
/// Corresponds to Python SDK's `PaymentPayloadV1` in `schemas/v1.py`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentPayloadV1 {
    /// Protocol version (always 1 for V1).
    #[serde(default = "default_v1")]
    pub x402_version: u32,

    /// Payment scheme identifier (at top level in V1).
    pub scheme: String,

    /// Network identifier (at top level in V1).
    pub network: Network,

    /// Scheme-specific payload data.
    pub payload: Value,
}

impl PaymentPayloadV1 {
    /// Returns the payment scheme.
    #[must_use]
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// Returns the network.
    #[must_use]
    pub fn network(&self) -> &str {
        &self.network
    }
}

/// V1 request to verify a payment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyRequestV1 {
    /// The payment payload to verify.
    pub payment_payload: PaymentPayloadV1,

    /// The requirements to verify against.
    pub payment_requirements: PaymentRequirementsV1,
}

/// V1 request to settle a payment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettleRequestV1 {
    /// The payment payload to settle.
    pub payment_payload: PaymentPayloadV1,

    /// The requirements for settlement.
    pub payment_requirements: PaymentRequirementsV1,
}

/// V1 supported response (legacy — no extensions or signers).
///
/// Corresponds to Python SDK's `SupportedResponseV1` in `schemas/v1.py`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportedResponseV1 {
    /// List of supported payment kinds.
    pub kinds: Vec<crate::SupportedKind>,
}

const fn default_v1() -> u32 {
    1
}
