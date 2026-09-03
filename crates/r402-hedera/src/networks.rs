//! Well-known Hedera network definitions and token deployments.

#[cfg(feature = "client")]
use std::str::FromStr;
use std::sync::LazyLock;

#[cfg(feature = "client")]
use r402_client::DefaultAssetInfo;
#[cfg(feature = "client")]
use r402_protocol::ChainId;
use r402_protocol::NetworkInfo;

use crate::chain::{
    HBAR_ASSET_ID, HBAR_DECIMALS, HEDERA_MAINNET_USDC, HEDERA_TESTNET_USDC, HederaAddress,
    HederaChainReference, HederaTokenDeployment, USDC_DECIMALS,
};

/// Well-known Hedera networks with their names and CAIP-2 identifiers.
pub static HEDERA_NETWORKS: &[NetworkInfo] = &[
    NetworkInfo {
        name: "hedera",
        namespace: "hedera",
        reference: "mainnet",
    },
    NetworkInfo {
        name: "hedera-testnet",
        namespace: "hedera",
        reference: "testnet",
    },
];

/// Parses a well-known Hedera entity id, panicking on malformed input.
#[allow(
    clippy::expect_used,
    reason = "hardcoded constant is infallible; validated by tests"
)]
fn well_known(s: &str) -> HederaAddress {
    s.parse().expect("well-known hedera address must be valid")
}

/// Native HBAR on both built-in networks.
static HBAR_DEPLOYMENTS: LazyLock<Vec<HederaTokenDeployment>> = LazyLock::new(|| {
    vec![
        HederaTokenDeployment::new(
            HederaChainReference::MAINNET,
            well_known(HBAR_ASSET_ID),
            HBAR_DECIMALS,
        ),
        HederaTokenDeployment::new(
            HederaChainReference::TESTNET,
            well_known(HBAR_ASSET_ID),
            HBAR_DECIMALS,
        ),
    ]
});

/// Well-known USDC token deployments on Hedera networks.
static USDC_DEPLOYMENTS: LazyLock<Vec<HederaTokenDeployment>> = LazyLock::new(|| {
    vec![
        HederaTokenDeployment::new(
            HederaChainReference::MAINNET,
            well_known(HEDERA_MAINNET_USDC),
            USDC_DECIMALS,
        ),
        HederaTokenDeployment::new(
            HederaChainReference::TESTNET,
            well_known(HEDERA_TESTNET_USDC),
            USDC_DECIMALS,
        ),
    ]
});

/// Returns all known USDC deployments on Hedera chains.
#[must_use]
pub fn usdc_hedera_deployments() -> &'static [HederaTokenDeployment] {
    &USDC_DEPLOYMENTS
}

/// Returns the USDC deployment for a specific Hedera chain, if known.
#[must_use]
pub fn usdc_hedera_deployment(
    chain: HederaChainReference,
) -> Option<&'static HederaTokenDeployment> {
    USDC_DEPLOYMENTS.iter().find(|d| d.chain_reference == chain)
}

/// Reverse lookup by HTS token id and CAIP-2 network.
#[cfg(feature = "client")]
#[must_use]
pub fn find_default_hedera_asset(asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
    if network.namespace() != "hedera" {
        return None;
    }
    let chain = HederaChainReference::from_str(network.reference()).ok()?;
    usdc_hedera_deployment(chain)
        .filter(|deployment| deployment.address.as_str() == asset)
        .map(|deployment| {
            DefaultAssetInfo::new(
                deployment.address.as_str(),
                u32::from(deployment.decimals),
                "USDC",
            )
        })
}

/// Returns the HBAR deployment for a specific Hedera chain.
#[must_use]
pub fn hbar_deployment(chain: HederaChainReference) -> Option<&'static HederaTokenDeployment> {
    HBAR_DEPLOYMENTS.iter().find(|d| d.chain_reference == chain)
}

/// Ergonomic accessors for USDC token deployments on well-known Hedera chains.
///
/// Combine with [`HederaTokenDeployment::amount`] for a fluent pricing API:
///
/// ```ignore
/// use r402_hedera::{HederaExact, USDC};
///
/// let tag = HederaExact::price_tag(pay_to, USDC::hedera_testnet().amount(1_000_000u64));
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
    pub fn on(chain: HederaChainReference) -> Option<&'static HederaTokenDeployment> {
        usdc_hedera_deployment(chain)
    }

    /// Returns all known USDC deployments on Hedera chains.
    #[must_use]
    pub fn all() -> &'static [HederaTokenDeployment] {
        usdc_hedera_deployments()
    }

    /// USDC on Hedera mainnet (`hedera:mainnet`).
    #[must_use]
    pub fn hedera() -> &'static HederaTokenDeployment {
        usdc_hedera_deployment(HederaChainReference::MAINNET)
            .expect("built-in USDC deployment for hedera mainnet missing")
    }

    /// USDC on Hedera testnet (`hedera:testnet`).
    #[must_use]
    pub fn hedera_testnet() -> &'static HederaTokenDeployment {
        usdc_hedera_deployment(HederaChainReference::TESTNET)
            .expect("built-in USDC deployment for hedera testnet missing")
    }
}

/// Ergonomic accessors for native HBAR on well-known Hedera chains.
#[derive(Debug, Clone, Copy)]
#[allow(
    clippy::upper_case_acronyms,
    reason = "HBAR is a well-known token ticker"
)]
pub struct HBAR;

#[allow(
    clippy::missing_panics_doc,
    clippy::expect_used,
    reason = "static deployment lookups are infallible for built-in data"
)]
impl HBAR {
    /// HBAR on Hedera mainnet (`hedera:mainnet`).
    #[must_use]
    pub fn hedera() -> &'static HederaTokenDeployment {
        hbar_deployment(HederaChainReference::MAINNET)
            .expect("built-in HBAR deployment for hedera mainnet missing")
    }

    /// HBAR on Hedera testnet (`hedera:testnet`).
    #[must_use]
    pub fn hedera_testnet() -> &'static HederaTokenDeployment {
        hbar_deployment(HederaChainReference::TESTNET)
            .expect("built-in HBAR deployment for hedera testnet missing")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::{HBAR_ASSET_ID, HEDERA_MAINNET_USDC, HEDERA_TESTNET_USDC};

    #[test]
    fn usdc_and_hbar_deployments_resolve() {
        assert_eq!(USDC::hedera().decimals, USDC_DECIMALS);
        assert_eq!(USDC::hedera_testnet().decimals, USDC_DECIMALS);
        assert_eq!(USDC::hedera().address.as_str(), HEDERA_MAINNET_USDC);
        assert_eq!(USDC::hedera_testnet().address.as_str(), HEDERA_TESTNET_USDC);
        assert_eq!(USDC::all().len(), 2);
        assert_eq!(HBAR::hedera().address.as_str(), HBAR_ASSET_ID);
        assert_eq!(HBAR::hedera().decimals, HBAR_DECIMALS);
        assert_eq!(HEDERA_NETWORKS.len(), 2);
    }

    #[cfg(feature = "client")]
    #[test]
    fn default_usdc_asset_lookup() {
        let network: ChainId = "hedera:mainnet".parse().unwrap();
        let info = find_default_hedera_asset(HEDERA_MAINNET_USDC, &network).unwrap();
        assert_eq!(info.symbol, "USDC");
        assert!(find_default_hedera_asset(HBAR_ASSET_ID, &network).is_none());
    }
}
