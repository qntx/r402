//! Well-known NEAR network definitions and token deployments.
//!
//! This module provides static network metadata and USDC NEP-141 token
//! deployment information for NEAR mainnet and testnet.

use std::sync::LazyLock;

use r402_core::chain::NetworkInfo;

use crate::DEFAULT_TOKEN_DECIMALS;
use crate::chain::{NearAddress, NearChainReference, NearTokenDeployment};

/// Well-known NEAR networks with their names and CAIP-2 identifiers.
pub static NEAR_NETWORKS: &[NetworkInfo] = &[
    NetworkInfo {
        name: "near",
        namespace: "near",
        reference: "mainnet",
    },
    NetworkInfo {
        name: "near-testnet",
        namespace: "near",
        reference: "testnet",
    },
];

/// Parses a well-known NEAR account id, panicking on malformed input.
///
/// Only used for hardcoded constants below; the address strings are fixed
/// and covered by tests, so a panic here indicates a build-time defect.
#[allow(
    clippy::expect_used,
    reason = "hardcoded constant is infallible; validated by tests"
)]
fn well_known(s: &str) -> NearAddress {
    s.parse().expect("well-known near address must be valid")
}

/// Well-known USDC token deployments on NEAR networks.
static USDC_DEPLOYMENTS: LazyLock<Vec<NearTokenDeployment>> = LazyLock::new(|| {
    vec![
        NearTokenDeployment::new(
            NearChainReference::MAINNET,
            well_known("17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1"),
            DEFAULT_TOKEN_DECIMALS,
        ),
        NearTokenDeployment::new(
            NearChainReference::TESTNET,
            well_known("3e2210e1184b45b64c8a434c0a7e7b23cc04ea7eb7a6c3c32520d03d4afcb8af"),
            DEFAULT_TOKEN_DECIMALS,
        ),
    ]
});

/// Returns all known USDC deployments on NEAR chains.
#[must_use]
pub fn usdc_near_deployments() -> &'static [NearTokenDeployment] {
    &USDC_DEPLOYMENTS
}

/// Returns the USDC deployment for a specific NEAR chain, if known.
#[must_use]
pub fn usdc_near_deployment(chain: NearChainReference) -> Option<&'static NearTokenDeployment> {
    USDC_DEPLOYMENTS.iter().find(|d| d.chain_reference == chain)
}

/// Ergonomic accessors for USDC token deployments on well-known NEAR chains.
///
/// Combine with [`NearTokenDeployment::amount`] for a fluent pricing API:
///
/// ```ignore
/// use r402_near::{NearExact, USDC};
///
/// let tag = NearExact::price_tag(pay_to, USDC::near_testnet().amount(1_000_000u128));
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
    pub fn on(chain: NearChainReference) -> Option<&'static NearTokenDeployment> {
        usdc_near_deployment(chain)
    }

    /// Returns all known USDC deployments on NEAR chains.
    #[must_use]
    pub fn all() -> &'static [NearTokenDeployment] {
        usdc_near_deployments()
    }

    /// USDC on NEAR mainnet (`near:mainnet`).
    #[must_use]
    pub fn near() -> &'static NearTokenDeployment {
        usdc_near_deployment(NearChainReference::MAINNET)
            .expect("built-in USDC deployment for near mainnet missing")
    }

    /// USDC on NEAR testnet (`near:testnet`).
    #[must_use]
    pub fn near_testnet() -> &'static NearTokenDeployment {
        usdc_near_deployment(NearChainReference::TESTNET)
            .expect("built-in USDC deployment for near testnet missing")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn usdc_deployments_resolve() {
        assert_eq!(USDC::near().decimals, DEFAULT_TOKEN_DECIMALS);
        assert_eq!(USDC::near_testnet().decimals, DEFAULT_TOKEN_DECIMALS);
        assert_eq!(USDC::all().len(), 2);
        assert_eq!(NEAR_NETWORKS.len(), 2);
    }
}
