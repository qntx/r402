//! Well-known Solana network definitions and token deployments.
//!
//! This module provides static network metadata and USDC token deployment
//! information for Solana mainnet and devnet.

use std::sync::LazyLock;

use r402::networks::NetworkInfo;
use solana_pubkey::pubkey;

use crate::chain::{SolanaChainReference, SolanaTokenDeployment};

/// Well-known Solana networks with their names and CAIP-2 identifiers.
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
pub fn usdc_solana_deployment(
    chain: &SolanaChainReference,
) -> Option<&'static SolanaTokenDeployment> {
    USDC_DEPLOYMENTS
        .iter()
        .find(|d| d.chain_reference == *chain)
}

/// Ergonomic accessors for USDC token deployments on well-known Solana chains.
///
/// Provides named methods for each supported chain, returning a static
/// reference to the deployment metadata. Combine with
/// [`SolanaTokenDeployment::amount`] for a fluent pricing API:
///
/// ```ignore
/// use r402_svm::{SolanaExact, USDC};
///
/// let tag = SolanaExact::price_tag(pay_to, USDC::solana().amount(1_000_000u64));
/// ```
#[derive(Debug, Clone, Copy)]
pub struct USDC;

#[allow(clippy::doc_markdown, clippy::missing_panics_doc)]
impl USDC {
    /// Looks up a USDC deployment by chain reference.
    ///
    /// Returns `None` if the chain is not in the built-in deployment table.
    #[must_use]
    pub fn on(chain: &SolanaChainReference) -> Option<&'static SolanaTokenDeployment> {
        usdc_solana_deployment(chain)
    }

    /// Returns all known USDC deployments on Solana chains.
    #[must_use]
    pub fn all() -> &'static [SolanaTokenDeployment] {
        usdc_solana_deployments()
    }

    /// USDC on Solana mainnet (solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp).
    #[must_use]
    pub fn solana() -> &'static SolanaTokenDeployment {
        usdc_solana_deployment(&SolanaChainReference::SOLANA)
            .expect("built-in USDC deployment for Solana mainnet missing")
    }

    /// USDC on Solana devnet (solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1).
    #[must_use]
    pub fn solana_devnet() -> &'static SolanaTokenDeployment {
        usdc_solana_deployment(&SolanaChainReference::SOLANA_DEVNET)
            .expect("built-in USDC deployment for Solana devnet missing")
    }
}
