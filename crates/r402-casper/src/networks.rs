//! Well-known Casper network definitions and wCSPR deployments.

#[cfg(feature = "client")]
use std::str::FromStr;
use std::sync::LazyLock;

#[cfg(feature = "client")]
use r402_client::DefaultAssetInfo;
#[cfg(feature = "client")]
use r402_protocol::ChainId;
use r402_protocol::NetworkInfo;

use crate::chain::motes::CSPR_DECIMALS;
use crate::chain::{CasperChainReference, CasperTokenDeployment, ContractPackageHash};

/// Well-known Casper networks with their names and CAIP-2 identifiers.
pub static CASPER_NETWORKS: &[NetworkInfo] = &[
    NetworkInfo {
        name: "casper",
        namespace: "casper",
        reference: "casper",
    },
    NetworkInfo {
        name: "casper-test",
        namespace: "casper",
        reference: "casper-test",
    },
];

/// EIP-712 domain `name` of the wCSPR CEP-18 contract.
pub const WCSPR_NAME: &str = "Wrapped CSPR";

/// EIP-712 domain `version` of the wCSPR CEP-18 contract.
pub const WCSPR_VERSION: &str = "1";

/// wCSPR testnet package hash.
///
/// Verify: <https://testnet.cspr.live/contract-package/3d80df21ba4ee4d66a2a1f60c32570dd5685e4b279f6538162a5fd1314847c1e>
const WCSPR_TESTNET_PACKAGE: &str =
    "3d80df21ba4ee4d66a2a1f60c32570dd5685e4b279f6538162a5fd1314847c1e";

/// Well-known wCSPR deployments on Casper networks.
#[allow(
    clippy::expect_used,
    reason = "hardcoded package hashes are validated at crate build time by the unit tests"
)]
static WCSPR_DEPLOYMENTS: LazyLock<Vec<CasperTokenDeployment>> = LazyLock::new(|| {
    vec![CasperTokenDeployment::new(
        CasperChainReference::CASPER_TEST,
        WCSPR_TESTNET_PACKAGE
            .parse::<ContractPackageHash>()
            .expect("built-in wCSPR testnet package hash is valid"),
        CSPR_DECIMALS,
        WCSPR_NAME,
        WCSPR_VERSION,
    )]
});

/// Returns all known wCSPR deployments on Casper chains.
#[must_use]
pub fn wcspr_casper_deployments() -> &'static [CasperTokenDeployment] {
    &WCSPR_DEPLOYMENTS
}

/// Returns the wCSPR deployment for a specific Casper chain, if known.
#[must_use]
pub fn wcspr_casper_deployment(
    chain: CasperChainReference,
) -> Option<&'static CasperTokenDeployment> {
    WCSPR_DEPLOYMENTS
        .iter()
        .find(|deployment| deployment.chain_reference == chain)
}

/// Reverse lookup by CEP-18 package hash and CAIP-2 network.
#[cfg(feature = "client")]
#[must_use]
pub fn find_default_casper_asset(asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
    if network.namespace() != "casper" {
        return None;
    }
    let chain = CasperChainReference::from_str(network.reference()).ok()?;
    let package: ContractPackageHash = asset.parse().ok()?;
    wcspr_casper_deployment(chain)
        .filter(|deployment| deployment.address == package)
        .map(|deployment| {
            DefaultAssetInfo::new(
                deployment.address.to_string(),
                u32::from(deployment.decimals),
                "wCSPR",
            )
        })
}

/// Ergonomic accessors for wCSPR token deployments on well-known Casper chains.
#[derive(Debug, Clone, Copy)]
#[allow(
    clippy::upper_case_acronyms,
    reason = "WCSPR is a well-known token ticker"
)]
pub struct WCSPR;

#[allow(
    clippy::missing_panics_doc,
    clippy::expect_used,
    reason = "static deployment lookups are infallible for built-in data"
)]
impl WCSPR {
    /// Looks up a wCSPR deployment by chain reference.
    #[must_use]
    pub fn on(chain: CasperChainReference) -> Option<&'static CasperTokenDeployment> {
        wcspr_casper_deployment(chain)
    }

    /// Returns all known wCSPR deployments on Casper chains.
    #[must_use]
    pub fn all() -> &'static [CasperTokenDeployment] {
        wcspr_casper_deployments()
    }

    /// wCSPR on Casper testnet (`casper:casper-test`).
    #[must_use]
    pub fn casper_test() -> &'static CasperTokenDeployment {
        wcspr_casper_deployment(CasperChainReference::CASPER_TEST)
            .expect("built-in wCSPR deployment for Casper testnet missing")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::CasperChainReference;

    #[test]
    fn network_table_matches_caip2_ids() {
        let ids: Vec<String> = CASPER_NETWORKS
            .iter()
            .map(|net| net.chain_id().to_string())
            .collect();
        assert_eq!(ids, vec!["casper:casper", "casper:casper-test"]);
    }

    #[test]
    fn every_network_info_maps_to_a_chain_reference() {
        for network in CASPER_NETWORKS {
            let chain_id = network.chain_id();
            assert!(
                CasperChainReference::try_from(chain_id).is_ok(),
                "network {} has no chain reference",
                network.name
            );
        }
    }

    #[test]
    fn builtin_wcspr_deployment_is_well_formed() {
        let deployment = WCSPR::casper_test();
        assert_eq!(
            deployment.chain_reference,
            CasperChainReference::CASPER_TEST
        );
        assert_eq!(deployment.decimals, CSPR_DECIMALS);
        assert_eq!(deployment.name, WCSPR_NAME);
        assert_eq!(deployment.version, WCSPR_VERSION);
        assert_eq!(deployment.address.to_string(), WCSPR_TESTNET_PACKAGE);
    }

    #[cfg(feature = "client")]
    #[test]
    fn find_default_asset_resolves_testnet_wcspr() {
        let network: ChainId = "casper:casper-test".parse().unwrap();
        let info = find_default_casper_asset(WCSPR_TESTNET_PACKAGE, &network).unwrap();
        assert_eq!(info.symbol, "wCSPR");
        assert_eq!(info.decimals, u32::from(CSPR_DECIMALS));
    }

    #[test]
    fn lookup_returns_none_for_chains_without_deployments() {
        assert!(WCSPR::on(CasperChainReference::CASPER).is_none());
        assert_eq!(WCSPR::all().len(), 1);
    }

    #[test]
    fn deployment_amount_is_exact() {
        let amount = WCSPR::casper_test().amount(1_000_000_000);
        assert_eq!(amount.amount.inner(), 1_000_000_000);
        assert_eq!(amount.amount.to_cspr_string(), "1");
    }
}
