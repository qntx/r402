//! Blockchain network identification.
//!
//! This module provides abstract types for blockchain network metadata.
//!
//! Concrete network data lives in chain-specific crates:
//!
//! - `r402-evm` provides [`EVM_NETWORKS`](r402_evm::EVM_NETWORKS) for EIP-155 chains
//! - `r402-svm` provides [`SOLANA_NETWORKS`](r402_svm::SOLANA_NETWORKS) for Solana chains

use crate::chain::ChainId;

/// A known network definition with its chain ID and human-readable name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkInfo {
    /// Human-readable network name (e.g., "base-sepolia", "solana")
    pub name: &'static str,
    /// CAIP-2 namespace (e.g., "eip155", "solana")
    pub namespace: &'static str,
    /// Chain reference (e.g., "84532" for Base Sepolia, "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp" for Solana mainnet)
    pub reference: &'static str,
}

impl NetworkInfo {
    /// Create a `ChainId` from this network info
    #[must_use]
    pub fn chain_id(&self) -> ChainId {
        ChainId::new(self.namespace, self.reference)
    }
}
