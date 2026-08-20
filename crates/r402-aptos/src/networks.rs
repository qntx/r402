//! Well-known Aptos network definitions and token deployments.

use std::sync::LazyLock;

use r402_core::chain::NetworkInfo;

use crate::DEFAULT_TOKEN_DECIMALS;
use crate::chain::{
    AptosAddress, AptosChainReference, AptosTokenDeployment, USDC_MAINNET_FA, USDC_TESTNET_FA,
};

/// Well-known Aptos networks with their names and CAIP-2 identifiers.
pub static APTOS_NETWORKS: &[NetworkInfo] = &[
    NetworkInfo {
        name: "aptos",
        namespace: "aptos",
        reference: "1",
    },
    NetworkInfo {
        name: "aptos-testnet",
        namespace: "aptos",
        reference: "2",
    },
];

/// Parses a well-known Aptos address, panicking on malformed input.
#[allow(
    clippy::expect_used,
    reason = "hardcoded constant is infallible; validated by tests"
)]
fn well_known(s: &str) -> AptosAddress {
    s.parse().expect("well-known aptos address must be valid")
}

/// Well-known USDC FA deployments on Aptos networks.
static USDC_DEPLOYMENTS: LazyLock<Vec<AptosTokenDeployment>> = LazyLock::new(|| {
    vec![
        AptosTokenDeployment::new(
            AptosChainReference::MAINNET,
            well_known(USDC_MAINNET_FA),
            DEFAULT_TOKEN_DECIMALS,
        ),
        AptosTokenDeployment::new(
            AptosChainReference::TESTNET,
            well_known(USDC_TESTNET_FA),
            DEFAULT_TOKEN_DECIMALS,
        ),
    ]
});

/// Returns all known USDC deployments on Aptos chains.
#[must_use]
pub fn usdc_aptos_deployments() -> &'static [AptosTokenDeployment] {
    &USDC_DEPLOYMENTS
}

/// Returns the USDC deployment for a specific Aptos chain, if known.
#[must_use]
pub fn usdc_aptos_deployment(chain: AptosChainReference) -> Option<&'static AptosTokenDeployment> {
    USDC_DEPLOYMENTS.iter().find(|d| d.chain_reference == chain)
}

/// Ergonomic accessors for USDC token deployments on well-known Aptos chains.
///
/// Combine with [`AptosTokenDeployment::amount`] for a fluent pricing API:
///
/// ```ignore
/// use r402_aptos::{AptosExact, USDC};
///
/// let tag = AptosExact::price_tag(pay_to, USDC::aptos_testnet().amount(1_000_000u64));
/// ```
#[derive(Debug, Clone, Copy)]
#[allow(
    clippy::upper_case_acronyms,
    reason = "USDC is a well-known token ticker"
)]
pub struct USDC;

#[allow(
    clippy::missing_panics_doc,
    clippy::expect_used,
    reason = "static deployment lookups are infallible for built-in data"
)]
impl USDC {
    /// Looks up a USDC deployment by chain reference.
    #[must_use]
    pub fn on(chain: AptosChainReference) -> Option<&'static AptosTokenDeployment> {
        usdc_aptos_deployment(chain)
    }

    /// Returns all known USDC deployments on Aptos chains.
    #[must_use]
    pub fn all() -> &'static [AptosTokenDeployment] {
        usdc_aptos_deployments()
    }

    /// USDC on Aptos mainnet (`aptos:1`).
    #[must_use]
    pub fn aptos() -> &'static AptosTokenDeployment {
        usdc_aptos_deployment(AptosChainReference::MAINNET)
            .expect("built-in USDC deployment for aptos mainnet missing")
    }

    /// USDC on Aptos testnet (`aptos:2`).
    #[must_use]
    pub fn aptos_testnet() -> &'static AptosTokenDeployment {
        usdc_aptos_deployment(AptosChainReference::TESTNET)
            .expect("built-in USDC deployment for aptos testnet missing")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn usdc_deployments_resolve() {
        assert_eq!(USDC::aptos().decimals, DEFAULT_TOKEN_DECIMALS);
        assert_eq!(USDC::aptos_testnet().decimals, DEFAULT_TOKEN_DECIMALS);
        assert_eq!(USDC::aptos().address.as_str(), USDC_MAINNET_FA);
        assert_eq!(USDC::aptos_testnet().address.as_str(), USDC_TESTNET_FA);
        assert_eq!(USDC::all().len(), 2);
        assert_eq!(APTOS_NETWORKS.len(), 2);
    }
}
