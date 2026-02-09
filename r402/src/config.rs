//! Configuration types for the x402 payment protocol.
//!
//! Provides configuration for protected resources.
//!
//! Corresponds to Python SDK's `schemas/config.py`.

use crate::proto::Network;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Configuration for a protected resource.
///
/// Defines what a resource server charges for a specific endpoint.
///
/// Corresponds to Python SDK's `ResourceConfig`.
///
/// # Example
///
/// ```rust
/// use r402::config::ResourceConfig;
///
/// let config = ResourceConfig {
///     scheme: "exact".into(),
///     pay_to: "0xRecipient".into(),
///     price: serde_json::json!("1.50"),
///     network: "eip155:8453".into(),
///     max_timeout_seconds: Some(300),
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceConfig {
    /// Payment scheme identifier (e.g., `"exact"`).
    pub scheme: String,

    /// Recipient address.
    pub pay_to: String,

    /// Price for the resource — can be a money string (`"1.50"`) or an
    /// [`AssetAmount`](crate::scheme::AssetAmount) object.
    pub price: Value,

    /// CAIP-2 network identifier (e.g., `"eip155:8453"`).
    pub network: Network,

    /// Maximum time in seconds for payment validity.
    /// Defaults to 300 if not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_timeout_seconds: Option<u64>,
}
