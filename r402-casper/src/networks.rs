//! Well-known Casper network definitions and token deployments.
//!
//! This module provides static network metadata and the wCSPR (CEP-18)
//! deployments r402 ships with.

use std::sync::LazyLock;

use r402_core::chain::NetworkInfo;

use crate::chain::{CasperChainReference, CasperTokenDeployment, ContractPackageHash};
use crate::motes::CSPR_DECIMALS;

/// Well-known Casper networks with their names and CAIP-2 identifiers.
///
/// Casper uses its chain name as the CAIP-2 reference, so the `name` and
/// `reference` fields coincide.
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
///
/// Only networks with an x402-enabled CEP-18 deployment appear here. Chains
/// without a built-in entry are still fully usable — construct a
/// [`CasperTokenDeployment`] directly with the package hash you operate.
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
    chain: &CasperChainReference,
) -> Option<&'static CasperTokenDeployment> {
    WCSPR_DEPLOYMENTS
        .iter()
        .find(|deployment| deployment.chain_reference == *chain)
}

/// Ergonomic accessors for wCSPR token deployments on well-known Casper
/// chains.
///
/// Combine with [`CasperTokenDeployment::amount`] for a fluent pricing API:
///
/// ```ignore
/// use r402_casper::{CasperExact, WCSPR};
///
/// let tag = CasperExact::price_tag(pay_to, WCSPR::casper_test().amount(1_000_000_000));
/// ```
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
    ///
    /// Returns `None` if the chain is not in the built-in deployment table.
    #[must_use]
    pub fn on(chain: &CasperChainReference) -> Option<&'static CasperTokenDeployment> {
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
        wcspr_casper_deployment(&CasperChainReference::CASPER_TEST)
            .expect("built-in wCSPR deployment for Casper testnet missing")
    }
}

#[cfg(test)]
mod tests {
    use r402_core::chain::ChainId;

    use super::*;

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
            let chain_id: ChainId = network.chain_id();
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

    #[test]
    fn lookup_returns_none_for_chains_without_deployments() {
        assert!(WCSPR::on(&CasperChainReference::CASPER).is_none());
        assert_eq!(WCSPR::all().len(), 1);
    }

    #[test]
    fn deployment_amount_is_exact() {
        let amount = WCSPR::casper_test().amount(1_000_000_000);
        assert_eq!(amount.amount.inner(), 1_000_000_000);
        assert_eq!(amount.amount.to_cspr_string(), "1");
    }
}
