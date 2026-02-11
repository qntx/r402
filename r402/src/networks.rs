//! Blockchain network identification and registry.
//!
//! This module provides abstract types for mapping V1 human-readable network
//! names (e.g., `"base"`) to CAIP-2 chain identifiers (e.g., `eip155:8453`).
//!
//! Concrete network data lives in chain-specific crates:
//!
//! - `r402-evm` provides [`EVM_NETWORKS`](r402_evm::EVM_NETWORKS) for EIP-155 chains
//! - `r402-svm` provides [`SOLANA_NETWORKS`](r402_svm::SOLANA_NETWORKS) for Solana chains
//!
//! Applications assemble a [`NetworkRegistry`] from these slices at startup.

use std::collections::HashMap;

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

/// Registry that maps V1 network names to [`ChainId`] values and vice versa.
///
/// Built from one or more `&[NetworkInfo]` slices provided by chain-specific
/// crates. This is the **single source of truth** for V1 name ↔ CAIP-2 lookups.
///
/// # Example
///
/// ```ignore
/// use r402::networks::NetworkRegistry;
///
/// let registry = NetworkRegistry::from_networks(r402_evm::EVM_NETWORKS)
///     .with_networks(r402_svm::SOLANA_NETWORKS);
///
/// let chain_id = registry.chain_id_by_name("base").unwrap();
/// let name = registry.name_by_chain_id(chain_id).unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct NetworkRegistry {
    name_to_chain_id: HashMap<&'static str, ChainId>,
    chain_id_to_name: HashMap<ChainId, &'static str>,
}

impl NetworkRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            name_to_chain_id: HashMap::new(),
            chain_id_to_name: HashMap::new(),
        }
    }

    /// Creates a registry pre-populated from a network info slice.
    #[must_use]
    pub fn from_networks(networks: &[NetworkInfo]) -> Self {
        let mut registry = Self::with_capacity(networks.len());
        registry.register(networks);
        registry
    }

    /// Creates an empty registry with pre-allocated capacity.
    #[must_use]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            name_to_chain_id: HashMap::with_capacity(cap),
            chain_id_to_name: HashMap::with_capacity(cap),
        }
    }

    /// Registers additional networks into this registry.
    pub fn register(&mut self, networks: &[NetworkInfo]) {
        for info in networks {
            self.name_to_chain_id.insert(info.name, info.chain_id());
            self.chain_id_to_name.insert(info.chain_id(), info.name);
        }
    }

    /// Builder-style method: registers additional networks and returns `self`.
    #[must_use]
    pub fn with_networks(mut self, networks: &[NetworkInfo]) -> Self {
        self.register(networks);
        self
    }

    /// Looks up a [`ChainId`] by its V1 human-readable network name.
    #[must_use]
    pub fn chain_id_by_name(&self, name: &str) -> Option<&ChainId> {
        self.name_to_chain_id.get(name)
    }

    /// Looks up a V1 human-readable network name by its [`ChainId`].
    #[must_use]
    pub fn name_by_chain_id(&self, chain_id: &ChainId) -> Option<&'static str> {
        self.chain_id_to_name.get(chain_id).copied()
    }

    /// Returns the number of registered networks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.name_to_chain_id.len()
    }

    /// Returns `true` if no networks are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.name_to_chain_id.is_empty()
    }
}

impl Default for NetworkRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Marker struct for USDC token deployment implementations.
///
/// This struct is used as a type parameter for chain-specific traits (e.g., `KnownNetworkEip155`,
/// `KnownNetworkSolana`) to provide per-network USDC token deployment information.
///
/// # Usage
///
/// Chain-specific crates implement traits for this marker struct to provide USDC token
/// deployments on different networks. For example:
///
/// - `r402-evm` implements `KnownNetworkEip155<Eip155TokenDeployment>` for `USDC`
/// - `r402-svm` implements `KnownNetworkSolana<SolanaTokenDeployment>` for `USDC`
#[derive(Debug, Clone, Copy)]
#[allow(clippy::upper_case_acronyms)]
pub struct USDC;

/// Marker struct for USDM (`MegaUSD`) token deployment implementations.
///
/// `MegaETH` uses USDM as its default stablecoin instead of USDC.
/// This marker enables chain-specific crates to provide USDM deployment data.
#[derive(Debug, Clone, Copy)]
#[allow(clippy::upper_case_acronyms)]
pub struct USDM;
