//! Facilitator capability discovery types.
//!
//! Types returned by a facilitator's `/supported` endpoint to advertise
//! which payment schemes, networks, and protocol versions it can handle.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_with::{VecSkipError, serde_as};

use crate::chain::ChainId;

/// Describes a payment method supported by a facilitator.
///
/// This type is returned in the [`SupportedResponse`] to indicate what
/// payment schemes, networks, and protocol versions a facilitator can handle.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportedPaymentKind {
    /// The x402 protocol version.
    pub x402_version: u8,
    /// The payment scheme identifier (e.g., "exact").
    pub scheme: String,
    /// The network identifier (CAIP-2 chain ID).
    pub network: String,
    /// Optional scheme-specific extra data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

/// Response from a facilitator's `/supported` endpoint.
///
/// This response tells clients what payment methods the facilitator supports,
/// including protocol versions, schemes, networks, and signer addresses.
#[serde_as]
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportedResponse {
    /// List of supported payment kinds.
    #[serde_as(as = "VecSkipError<_>")]
    pub kinds: Vec<SupportedPaymentKind>,
    /// List of supported protocol extensions.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Map of CAIP-2 patterns to signer addresses.
    ///
    /// Keys can be exact chain IDs (e.g., `"eip155:8453"`) or wildcard patterns
    /// (e.g., `"eip155:*"`), matching the official x402 wire format.
    #[serde(default)]
    pub signers: HashMap<String, Vec<String>>,
}

impl SupportedResponse {
    /// Finds signer addresses that match the given chain ID.
    ///
    /// Checks both exact match (e.g., `"eip155:8453"`) and namespace wildcard
    /// (e.g., `"eip155:*"`).
    #[must_use]
    pub fn signers_for_chain(&self, chain_id: &ChainId) -> Vec<&str> {
        let exact_key = chain_id.to_string();
        let wildcard_key = format!("{}:*", chain_id.namespace());

        let mut result = Vec::new();
        if let Some(addrs) = self.signers.get(&exact_key) {
            result.extend(addrs.iter().map(String::as_str));
        }
        if let Some(addrs) = self.signers.get(&wildcard_key) {
            result.extend(addrs.iter().map(String::as_str));
        }
        result
    }
}
