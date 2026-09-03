//! Well-known Keeta network definitions and token deployments.

#[cfg(feature = "client")]
use std::str::FromStr;
use std::sync::LazyLock;

#[cfg(feature = "client")]
use r402_client::DefaultAssetInfo;
#[cfg(feature = "client")]
use r402_protocol::ChainId;
use r402_protocol::NetworkInfo;

use crate::chain::{
    DEFAULT_TOKEN_DECIMALS, KeetaAddress, KeetaChainReference, KeetaTokenDeployment,
};

/// Well-known Keeta networks with their names and CAIP-2 identifiers.
pub static KEETA_NETWORKS: &[NetworkInfo] = &[
    NetworkInfo {
        name: "keeta",
        namespace: "keeta",
        reference: "21378",
    },
    NetworkInfo {
        name: "keeta-testnet",
        namespace: "keeta",
        reference: "1413829460",
    },
];

/// Parses a well-known Keeta address, panicking on malformed input.
///
/// Only used for hardcoded constants below; the address strings are fixed
/// and covered by tests, so a panic here indicates a build-time defect.
#[allow(
    clippy::expect_used,
    reason = "hardcoded constant is infallible; validated by tests"
)]
fn well_known(s: &str) -> KeetaAddress {
    s.parse().expect("well-known keeta address must be valid")
}

/// Well-known USDC token deployments on Keeta networks.
static USDC_DEPLOYMENTS: LazyLock<Vec<KeetaTokenDeployment>> = LazyLock::new(|| {
    vec![
        KeetaTokenDeployment::new(
            KeetaChainReference::MAINNET,
            well_known("keeta_amnkge74xitii5dsobstldatv3irmyimujfjotftx7plaaaseam4bntb7wnna"),
            DEFAULT_TOKEN_DECIMALS,
        ),
        KeetaTokenDeployment::new(
            KeetaChainReference::TESTNET,
            well_known("keeta_apna75yhhvnv4ei7ape55hndk4yepno7a7i2mhtiwahiygixjcnmvswxhnmnk"),
            DEFAULT_TOKEN_DECIMALS,
        ),
    ]
});

/// Returns all known USDC deployments on Keeta chains.
#[must_use]
pub fn usdc_keeta_deployments() -> &'static [KeetaTokenDeployment] {
    &USDC_DEPLOYMENTS
}

/// Returns the USDC deployment for a specific Keeta chain, if known.
#[must_use]
pub fn usdc_keeta_deployment(chain: KeetaChainReference) -> Option<&'static KeetaTokenDeployment> {
    USDC_DEPLOYMENTS.iter().find(|d| d.chain_reference == chain)
}

/// Reverse lookup by token account and CAIP-2 network.
#[cfg(feature = "client")]
#[must_use]
pub fn find_default_keeta_asset(asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
    if network.namespace() != "keeta" {
        return None;
    }
    let chain = KeetaChainReference::from_str(network.reference()).ok()?;
    let address: KeetaAddress = asset.parse().ok()?;
    usdc_keeta_deployment(chain)
        .filter(|deployment| deployment.address == address)
        .map(|deployment| {
            DefaultAssetInfo::new(
                deployment.address.to_string(),
                u32::from(deployment.decimals),
                "USDC",
            )
        })
}

/// Ergonomic accessors for USDC token deployments on well-known Keeta chains.
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
    pub fn on(chain: KeetaChainReference) -> Option<&'static KeetaTokenDeployment> {
        usdc_keeta_deployment(chain)
    }

    /// Returns all known USDC deployments on Keeta chains.
    #[must_use]
    pub fn all() -> &'static [KeetaTokenDeployment] {
        usdc_keeta_deployments()
    }

    /// USDC on Keeta mainnet (`keeta:21378`).
    #[must_use]
    pub fn keeta() -> &'static KeetaTokenDeployment {
        usdc_keeta_deployment(KeetaChainReference::MAINNET)
            .expect("built-in USDC deployment for keeta mainnet missing")
    }

    /// USDC on Keeta testnet (`keeta:1413829460`).
    #[must_use]
    pub fn keeta_testnet() -> &'static KeetaTokenDeployment {
        usdc_keeta_deployment(KeetaChainReference::TESTNET)
            .expect("built-in USDC deployment for keeta testnet missing")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usdc_deployments_resolve() {
        assert_eq!(USDC::keeta().decimals, DEFAULT_TOKEN_DECIMALS);
        assert_eq!(USDC::keeta_testnet().decimals, DEFAULT_TOKEN_DECIMALS);
        assert_eq!(USDC::all().len(), 2);
        assert_eq!(KEETA_NETWORKS.len(), 2);
        assert_eq!(
            USDC::keeta().address.as_str(),
            "keeta_amnkge74xitii5dsobstldatv3irmyimujfjotftx7plaaaseam4bntb7wnna"
        );
        assert_eq!(
            USDC::keeta_testnet().address.as_str(),
            "keeta_apna75yhhvnv4ei7ape55hndk4yepno7a7i2mhtiwahiygixjcnmvswxhnmnk"
        );
    }
}
