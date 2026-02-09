//! EVM chain primitives.
//!
//! Provides core types for working with EIP-155 chains, including chain
//! references, token deployments, and asset information.

use alloy_primitives::Address;
use serde::{Deserialize, Serialize};

/// An EIP-155 chain ID (e.g., 8453 for Base, 137 for Polygon).
pub type ChainId = u64;

/// Formats a chain ID as a CAIP-2 identifier.
///
/// Example: `caip2(8453)` returns `"eip155:8453"`.
#[must_use]
pub fn caip2(chain_id: ChainId) -> String {
    format!("eip155:{chain_id}")
}

/// Parses a CAIP-2 identifier into an EIP-155 chain ID.
///
/// Returns `None` if the input is not a valid `eip155:` prefixed string.
#[must_use]
pub fn parse_caip2(caip: &str) -> Option<ChainId> {
    caip.strip_prefix("eip155:").and_then(|s| s.parse().ok())
}

/// A token deployment on an EVM network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenDeployment {
    /// Contract address.
    pub address: Address,
    /// Number of decimals (e.g., 6 for USDC).
    pub decimals: u8,
}

/// Asset information for a token on a specific network.
///
/// Corresponds to Python SDK's `AssetInfo` in `mechanisms/evm/constants.py`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetInfo {
    /// Contract address.
    pub address: Address,
    /// Number of decimals.
    pub decimals: u8,
    /// EIP-712 domain name for the token contract.
    pub name: String,
    /// EIP-712 domain version for the token contract.
    pub version: String,
}

/// Configuration for a known EVM network.
///
/// Corresponds to Python SDK's `NetworkConfig` in `mechanisms/evm/constants.py`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkConfig {
    /// CAIP-2 network identifier (e.g., `"eip155:8453"`).
    pub network: String,
    /// EIP-155 chain ID.
    pub chain_id: ChainId,
    /// Map of asset addresses to their info.
    pub assets: Vec<AssetInfo>,
}

impl NetworkConfig {
    /// Finds an asset by its contract address (case-insensitive).
    #[must_use]
    pub fn find_asset(&self, address: Address) -> Option<&AssetInfo> {
        self.assets.iter().find(|a| a.address == address)
    }
}
