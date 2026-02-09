//! Facilitator response types for the x402 protocol.
//!
//! These types are used for communication between resource servers and
//! facilitators during payment verification and settlement.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Network;

/// Response from payment verification.
///
/// Corresponds to Python SDK's `VerifyResponse` in `schemas/responses.py`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyResponse {
    /// Whether the payment is valid.
    pub is_valid: bool,

    /// Machine-readable reason for invalidity (if `is_valid` is false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalid_reason: Option<String>,

    /// Human-readable message for invalidity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalid_message: Option<String>,

    /// The payer's address (if known).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payer: Option<String>,
}

impl VerifyResponse {
    /// Creates a valid verification response.
    #[must_use]
    pub fn valid(payer: impl Into<String>) -> Self {
        Self {
            is_valid: true,
            invalid_reason: None,
            invalid_message: None,
            payer: Some(payer.into()),
        }
    }

    /// Creates an invalid verification response.
    #[must_use]
    pub fn invalid(reason: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            is_valid: false,
            invalid_reason: Some(reason.into()),
            invalid_message: Some(message.into()),
            payer: None,
        }
    }

    /// Creates an invalid response with a payer address.
    #[must_use]
    pub fn invalid_with_payer(
        reason: impl Into<String>,
        message: impl Into<String>,
        payer: impl Into<String>,
    ) -> Self {
        Self {
            is_valid: false,
            invalid_reason: Some(reason.into()),
            invalid_message: Some(message.into()),
            payer: Some(payer.into()),
        }
    }
}

/// Response from payment settlement.
///
/// Corresponds to Python SDK's `SettleResponse` in `schemas/responses.py`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettleResponse {
    /// Whether settlement was successful.
    pub success: bool,

    /// Machine-readable reason for failure (if `success` is false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,

    /// Human-readable message for failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,

    /// The payer's address (if known).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payer: Option<String>,

    /// Transaction hash/identifier.
    pub transaction: String,

    /// Network where settlement occurred.
    pub network: Network,
}

impl SettleResponse {
    /// Creates a successful settlement response.
    #[must_use]
    pub fn success(
        transaction: impl Into<String>,
        network: impl Into<String>,
        payer: impl Into<String>,
    ) -> Self {
        Self {
            success: true,
            error_reason: None,
            error_message: None,
            payer: Some(payer.into()),
            transaction: transaction.into(),
            network: network.into(),
        }
    }

    /// Creates a failed settlement response.
    #[must_use]
    pub fn error(
        reason: impl Into<String>,
        message: impl Into<String>,
        network: impl Into<String>,
    ) -> Self {
        Self {
            success: false,
            error_reason: Some(reason.into()),
            error_message: Some(message.into()),
            payer: None,
            transaction: String::new(),
            network: network.into(),
        }
    }
}

/// A supported payment configuration.
///
/// Describes a single (version, scheme, network) combination that
/// a facilitator supports.
///
/// Corresponds to Python SDK's `SupportedKind` in `schemas/responses.py`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportedKind {
    /// Protocol version for this kind (1 or 2).
    pub x402_version: u32,

    /// Payment scheme identifier (e.g., "exact").
    pub scheme: String,

    /// CAIP-2 network identifier (e.g., "eip155:8453").
    pub network: Network,

    /// Additional scheme-specific data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

/// Describes what payment kinds a facilitator supports.
///
/// Corresponds to Python SDK's `SupportedResponse` in `schemas/responses.py`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportedResponse {
    /// List of supported payment kinds.
    pub kinds: Vec<SupportedKind>,

    /// List of supported extension keys (e.g., `["bazaar"]`).
    #[serde(default)]
    pub extensions: Vec<String>,

    /// Map of CAIP family pattern to signer addresses.
    ///
    /// Example: `{"eip155:*": ["0xFacilitatorAddress"]}`
    #[serde(default)]
    pub signers: HashMap<String, Vec<String>>,
}

impl SupportedResponse {
    /// Creates a new `SupportedResponse`.
    #[must_use]
    pub const fn new(
        kinds: Vec<SupportedKind>,
        extensions: Vec<String>,
        signers: HashMap<String, Vec<String>>,
    ) -> Self {
        Self {
            kinds,
            extensions,
            signers,
        }
    }
}
