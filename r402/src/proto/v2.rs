//! V2 payment types for the x402 protocol.
//!
//! These types correspond to the current (V2) protocol version using CAIP-2
//! network identifiers and structured payment requirements.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Network;

/// Describes the resource being accessed.
///
/// Corresponds to Python SDK's `ResourceInfo` in `schemas/payments.py`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceInfo {
    /// The URL of the resource.
    pub url: String,

    /// Optional human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Optional MIME type of the resource.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// V2 payment requirements structure.
///
/// Defines what a resource server requires for payment, including scheme,
/// network, asset, amount, recipient, and timeout.
///
/// Corresponds to Python SDK's `PaymentRequirements` in `schemas/payments.py`.
///
/// # JSON Format
///
/// ```json
/// {
///   "scheme": "exact",
///   "network": "eip155:8453",
///   "asset": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
///   "amount": "1000000",
///   "payTo": "0x...",
///   "maxTimeoutSeconds": 300,
///   "extra": {}
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequirements {
    /// Payment scheme identifier (e.g., "exact").
    pub scheme: String,

    /// CAIP-2 network identifier (e.g., "eip155:8453").
    pub network: Network,

    /// Asset address/identifier (e.g., USDC contract address).
    pub asset: String,

    /// Amount in smallest unit (e.g., "1000000" for 1 USDC).
    pub amount: String,

    /// Recipient address.
    pub pay_to: String,

    /// Maximum time in seconds for payment validity.
    pub max_timeout_seconds: u64,

    /// Additional scheme-specific data (e.g., EIP-712 domain params).
    #[serde(default = "default_empty_object")]
    pub extra: Value,
}

impl PaymentRequirements {
    /// Returns the payment amount.
    #[must_use]
    pub fn amount(&self) -> &str {
        &self.amount
    }

    /// Returns the extra metadata, or `None` if it is null.
    #[must_use]
    pub fn extra(&self) -> Option<&Value> {
        if self.extra.is_null() {
            None
        } else {
            Some(&self.extra)
        }
    }
}

/// V2 402 response structure.
///
/// Sent by the resource server when payment is required. Contains the list of
/// accepted payment options and optional resource information.
///
/// Corresponds to Python SDK's `PaymentRequired` in `schemas/payments.py`.
///
/// # JSON Format
///
/// ```json
/// {
///   "x402Version": 2,
///   "error": null,
///   "resource": { "url": "/api/data", "description": "Market data" },
///   "accepts": [{ "scheme": "exact", "network": "eip155:8453", ... }],
///   "extensions": null
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequired {
    /// Protocol version (always 2 for V2).
    #[serde(default = "default_v2")]
    pub x402_version: u32,

    /// Optional error message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Optional resource information.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceInfo>,

    /// List of accepted payment requirements.
    pub accepts: Vec<PaymentRequirements>,

    /// Optional extension data (e.g., bazaar).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Value>,
}

/// V2 payment payload structure.
///
/// Sent by the client to fulfill a payment requirement. Contains the
/// scheme-specific payload and the accepted requirements.
///
/// Corresponds to Python SDK's `PaymentPayload` in `schemas/payments.py`.
///
/// # JSON Format
///
/// ```json
/// {
///   "x402Version": 2,
///   "payload": { "authorization": {...}, "signature": "0x..." },
///   "accepted": { "scheme": "exact", "network": "eip155:8453", ... },
///   "resource": null,
///   "extensions": null
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentPayload {
    /// Protocol version (always 2 for V2).
    #[serde(default = "default_v2")]
    pub x402_version: u32,

    /// Scheme-specific payload data.
    pub payload: Value,

    /// The payment requirements being fulfilled.
    pub accepted: PaymentRequirements,

    /// Optional resource information.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceInfo>,

    /// Optional extension data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Value>,
}

impl PaymentPayload {
    /// Returns the payment scheme from accepted requirements.
    #[must_use]
    pub fn scheme(&self) -> &str {
        &self.accepted.scheme
    }

    /// Returns the network from accepted requirements.
    #[must_use]
    pub fn network(&self) -> &str {
        &self.accepted.network
    }
}

/// Request to verify a payment.
///
/// Corresponds to Python SDK's `VerifyRequest` in `schemas/responses.py`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyRequest {
    /// The payment payload to verify.
    pub payment_payload: PaymentPayload,

    /// The requirements to verify against.
    pub payment_requirements: PaymentRequirements,
}

/// Request to settle a payment.
///
/// Corresponds to Python SDK's `SettleRequest` in `schemas/responses.py`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettleRequest {
    /// The payment payload to settle.
    pub payment_payload: PaymentPayload,

    /// The requirements for settlement.
    pub payment_requirements: PaymentRequirements,
}

const fn default_v2() -> u32 {
    2
}

fn default_empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}
