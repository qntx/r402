use std::sync::LazyLock;

use r402::networks::{NetworkInfo, NetworkRegistry};
use solana_pubkey::pubkey;

use crate::chain::{SolanaChainReference, SolanaTokenDeployment};

/// Well-known Solana networks with their V1 names and CAIP-2 identifiers.
///
/// This is the **single source of truth** for Solana network name ↔ chain ID mappings.
/// Use [`solana_network_registry()`] for convenient lookups, or pass this slice to
/// [`NetworkRegistry::from_networks()`] to build a combined cross-chain registry.
pub static SOLANA_NETWORKS: &[NetworkInfo] = &[
    NetworkInfo {
        name: "solana",
        namespace: "solana",
        reference: "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp",
    },
    NetworkInfo {
        name: "solana-devnet",
        namespace: "solana",
        reference: "EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
    },
];

static SOLANA_REGISTRY: LazyLock<NetworkRegistry> =
    LazyLock::new(|| NetworkRegistry::from_networks(SOLANA_NETWORKS));

/// Returns a lazily-initialized [`NetworkRegistry`] containing all known Solana networks.
///
/// This is a convenience accessor for V1 code paths within the `r402-svm` crate.
/// For cross-chain registries, build your own [`NetworkRegistry`] from [`SOLANA_NETWORKS`].
#[must_use]
pub fn solana_network_registry() -> &'static NetworkRegistry {
    &SOLANA_REGISTRY
}

/// Well-known USDC token deployments on Solana networks.
///
/// Use [`usdc_solana_deployment()`] for per-chain lookups, or [`usdc_solana_deployments()`]
/// to iterate over all known deployments.
static USDC_DEPLOYMENTS: LazyLock<Vec<SolanaTokenDeployment>> = LazyLock::new(|| {
    vec![
        // Solana mainnet — native Circle USDC (SPL Token)
        // Verify: https://solscan.io/token/EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v
        SolanaTokenDeployment::new(
            SolanaChainReference::SOLANA,
            pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").into(),
            6,
        ),
        // Solana devnet — native Circle USDC testnet (SPL Token)
        // Verify: https://explorer.solana.com/address/4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU?cluster=devnet
        SolanaTokenDeployment::new(
            SolanaChainReference::SOLANA_DEVNET,
            pubkey!("4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU").into(),
            6,
        ),
    ]
});

/// Returns all known USDC deployments on Solana chains.
#[must_use]
pub fn usdc_solana_deployments() -> &'static [SolanaTokenDeployment] {
    &USDC_DEPLOYMENTS
}

/// Returns the USDC deployment for a specific Solana chain, if known.
#[must_use]
pub fn usdc_solana_deployment(chain: &SolanaChainReference) -> Option<&'static SolanaTokenDeployment> {
    USDC_DEPLOYMENTS.iter().find(|d| d.chain_reference == *chain)
}
