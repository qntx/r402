//! Circle USDC deployments on Solana networks.

use std::sync::LazyLock;

use solana_pubkey::pubkey;

use crate::chain::{SolanaChainReference, SolanaTokenDeployment, TOKEN_PROGRAM_ID};

/// Well-known USDC token deployments on Solana networks.
static USDC_DEPLOYMENTS: LazyLock<Vec<SolanaTokenDeployment>> = LazyLock::new(|| {
    vec![
        // Solana mainnet — native Circle USDC (SPL Token)
        // Verify: https://solscan.io/token/EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v
        SolanaTokenDeployment::new(
            SolanaChainReference::SOLANA,
            pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").into(),
            6,
            TOKEN_PROGRAM_ID,
        ),
        // Solana devnet — native Circle USDC testnet (SPL Token)
        // Verify: https://explorer.solana.com/address/4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU?cluster=devnet
        SolanaTokenDeployment::new(
            SolanaChainReference::SOLANA_DEVNET,
            pubkey!("4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU").into(),
            6,
            TOKEN_PROGRAM_ID,
        ),
        // Testnet USDC uses the Circle devnet mint, matching official TS defaultAssets.
        SolanaTokenDeployment::new(
            SolanaChainReference::SOLANA_TESTNET,
            pubkey!("4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU").into(),
            6,
            TOKEN_PROGRAM_ID,
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
/// ```ignore
/// use r402_solana::{SolanaExact, USDC};
///
/// let tag = SolanaExact::price_tag(pay_to, USDC::solana().amount(1_000_000u64));
/// ```
#[derive(Debug, Clone, Copy)]
#[allow(
    clippy::upper_case_acronyms,
    reason = "USDC is a well-known token ticker"
)]
pub struct USDC;

#[allow(
    clippy::doc_markdown,
    clippy::missing_panics_doc,
    clippy::expect_used,
    reason = "static deployment lookups are infallible for built-in data"
)]
impl USDC {
    /// Looks up a USDC deployment by chain reference.
    #[must_use]
    pub fn on(chain: &SolanaChainReference) -> Option<&'static SolanaTokenDeployment> {
        usdc_solana_deployment(chain)
    }

    /// Returns all known USDC deployments on Solana chains.
    #[must_use]
    pub fn all() -> &'static [SolanaTokenDeployment] {
        usdc_solana_deployments()
    }

    /// USDC on Solana mainnet (`solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp`).
    #[must_use]
    pub fn solana() -> &'static SolanaTokenDeployment {
        usdc_solana_deployment(&SolanaChainReference::SOLANA)
            .expect("built-in USDC deployment for Solana mainnet missing")
    }

    /// USDC on Solana devnet (`solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1`).
    #[must_use]
    pub fn solana_devnet() -> &'static SolanaTokenDeployment {
        usdc_solana_deployment(&SolanaChainReference::SOLANA_DEVNET)
            .expect("built-in USDC deployment for Solana devnet missing")
    }

    /// USDC on Solana testnet (`solana:4uhcVJyU9pJkvQyS88uRDiswHXSCkY3z`).
    #[must_use]
    pub fn solana_testnet() -> &'static SolanaTokenDeployment {
        usdc_solana_deployment(&SolanaChainReference::SOLANA_TESTNET)
            .expect("built-in USDC deployment for Solana testnet missing")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::TOKEN_PROGRAM_ID;

    #[test]
    fn usdc_testnet_mint_equals_devnet() {
        assert_eq!(
            USDC::solana_testnet().address,
            USDC::solana_devnet().address
        );
        assert_eq!(USDC::solana().token_program, TOKEN_PROGRAM_ID);
        assert_eq!(USDC::solana_devnet().token_program, TOKEN_PROGRAM_ID);
        assert_eq!(USDC::solana_testnet().token_program, TOKEN_PROGRAM_ID);
        assert_eq!(USDC::all().len(), 3);
        assert_eq!(USDC::solana().decimals, 6);
    }
}
