//! Well-known Algorand network definitions and token deployments.

use std::str::FromStr;
use std::sync::LazyLock;

use r402_core::chain::{ChainId, NetworkInfo};
use r402_core::scheme::DefaultAssetInfo;

use crate::DEFAULT_TOKEN_DECIMALS;
use crate::chain::{AlgorandChainReference, AlgorandTokenDeployment};

/// Well-known Algorand networks with their names and CAIP-2 identifiers.
pub static ALGORAND_NETWORKS: &[NetworkInfo] = &[
    NetworkInfo {
        name: "algorand",
        namespace: "algorand",
        reference: "wGHE2Pwdvd7S12BL5FaOP20EGYesN73k",
    },
    NetworkInfo {
        name: "algorand-testnet",
        namespace: "algorand",
        reference: "SGO1GKSzyE7IEPItTxCByw9x8FmnrCDe",
    },
];

/// Mainnet USDC ASA id.
pub const USDC_MAINNET_ASA_ID: u64 = 31_566_704;

/// Testnet USDC ASA id.
pub const USDC_TESTNET_ASA_ID: u64 = 10_458_941;

/// Well-known USDC token deployments on Algorand networks.
static USDC_DEPLOYMENTS: LazyLock<Vec<AlgorandTokenDeployment>> = LazyLock::new(|| {
    vec![
        AlgorandTokenDeployment::new(
            AlgorandChainReference::MAINNET,
            USDC_MAINNET_ASA_ID,
            DEFAULT_TOKEN_DECIMALS,
        ),
        AlgorandTokenDeployment::new(
            AlgorandChainReference::TESTNET,
            USDC_TESTNET_ASA_ID,
            DEFAULT_TOKEN_DECIMALS,
        ),
    ]
});

/// Returns all known USDC deployments on Algorand chains.
#[must_use]
pub fn usdc_algorand_deployments() -> &'static [AlgorandTokenDeployment] {
    &USDC_DEPLOYMENTS
}

/// Returns the USDC deployment for a specific Algorand chain, if known.
#[must_use]
pub fn usdc_algorand_deployment(
    chain: AlgorandChainReference,
) -> Option<&'static AlgorandTokenDeployment> {
    USDC_DEPLOYMENTS.iter().find(|d| d.chain_reference == chain)
}

/// Reverse lookup by ASA id and CAIP-2 network.
#[must_use]
pub fn find_default_algorand_asset(asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
    if network.namespace() != "algorand" {
        return None;
    }
    let chain = AlgorandChainReference::from_str(network.reference()).ok()?;
    let asset_id: u64 = asset.parse().ok()?;
    usdc_algorand_deployment(chain)
        .filter(|deployment| deployment.asset_id == asset_id)
        .map(|deployment| {
            DefaultAssetInfo::new(
                deployment.asset_id.to_string(),
                u32::from(deployment.decimals),
                "USDC",
            )
        })
}

/// Ergonomic accessors for USDC token deployments on well-known Algorand chains.
///
/// Combine with [`AlgorandTokenDeployment::amount`] for a fluent pricing API:
///
/// ```ignore
/// use r402_algorand::{AlgorandExact, USDC};
///
/// let tag = AlgorandExact::price_tag(pay_to, USDC::algorand_testnet().amount(1_000_000u64));
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
    pub fn on(chain: AlgorandChainReference) -> Option<&'static AlgorandTokenDeployment> {
        usdc_algorand_deployment(chain)
    }

    /// Returns all known USDC deployments on Algorand chains.
    #[must_use]
    pub fn all() -> &'static [AlgorandTokenDeployment] {
        usdc_algorand_deployments()
    }

    /// USDC on Algorand mainnet (`algorand:wGHE2Pwdvd7S12BL5FaOP20EGYesN73k`).
    #[must_use]
    pub fn algorand() -> &'static AlgorandTokenDeployment {
        usdc_algorand_deployment(AlgorandChainReference::MAINNET)
            .expect("built-in USDC deployment for algorand mainnet missing")
    }

    /// USDC on Algorand testnet (`algorand:SGO1GKSzyE7IEPItTxCByw9x8FmnrCDe`).
    #[must_use]
    pub fn algorand_testnet() -> &'static AlgorandTokenDeployment {
        usdc_algorand_deployment(AlgorandChainReference::TESTNET)
            .expect("built-in USDC deployment for algorand testnet missing")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn usdc_deployments_resolve() {
        assert_eq!(USDC::algorand().decimals, DEFAULT_TOKEN_DECIMALS);
        assert_eq!(USDC::algorand().asset_id, USDC_MAINNET_ASA_ID);
        assert_eq!(USDC::algorand_testnet().asset_id, USDC_TESTNET_ASA_ID);
        assert_eq!(USDC::all().len(), 2);
        assert_eq!(ALGORAND_NETWORKS.len(), 2);
        let network: ChainId = "algorand:wGHE2Pwdvd7S12BL5FaOP20EGYesN73k".parse().unwrap();
        let info = find_default_algorand_asset(&USDC_MAINNET_ASA_ID.to_string(), &network).unwrap();
        assert_eq!(info.symbol, "USDC");
        assert_eq!(info.asset, USDC_MAINNET_ASA_ID.to_string());
    }
}
