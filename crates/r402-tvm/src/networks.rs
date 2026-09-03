//! Well-known TON network definitions and token deployments.

#[cfg(feature = "client")]
use std::str::FromStr;
use std::sync::LazyLock;

#[cfg(feature = "client")]
use r402_client::DefaultAssetInfo;
#[cfg(feature = "client")]
use r402_protocol::ChainId;
use r402_protocol::NetworkInfo;

use crate::chain::{TvmAddress, TvmChainReference, TvmTokenDeployment};
use crate::{DEFAULT_TOKEN_DECIMALS, USDT_MAINNET_MINTER, USDT_TESTNET_MINTER};

/// Well-known TON networks with their names and CAIP-2 identifiers.
pub static TVM_NETWORKS: &[NetworkInfo] = &[
    NetworkInfo {
        name: "ton",
        namespace: "tvm",
        reference: "-239",
    },
    NetworkInfo {
        name: "ton-testnet",
        namespace: "tvm",
        reference: "-3",
    },
];

/// Parses a well-known TON address, panicking on malformed input.
///
/// Only used for hardcoded constants below; the address strings are fixed
/// and covered by tests, so a panic here indicates a build-time defect.
#[allow(
    clippy::expect_used,
    reason = "hardcoded constant is infallible; validated by tests"
)]
fn well_known(s: &str) -> TvmAddress {
    s.parse().expect("well-known tvm address must be valid")
}

/// Well-known USDT jetton deployments on TON networks.
static USDT_DEPLOYMENTS: LazyLock<Vec<TvmTokenDeployment>> = LazyLock::new(|| {
    vec![
        TvmTokenDeployment::new(
            TvmChainReference::MAINNET,
            well_known(USDT_MAINNET_MINTER),
            DEFAULT_TOKEN_DECIMALS,
        ),
        TvmTokenDeployment::new(
            TvmChainReference::TESTNET,
            well_known(USDT_TESTNET_MINTER),
            DEFAULT_TOKEN_DECIMALS,
        ),
    ]
});

/// Returns all known USDT deployments on TON chains.
#[must_use]
pub fn usdt_tvm_deployments() -> &'static [TvmTokenDeployment] {
    &USDT_DEPLOYMENTS
}

/// Returns the USDT deployment for a specific TON chain, if known.
#[must_use]
pub fn usdt_tvm_deployment(chain: TvmChainReference) -> Option<&'static TvmTokenDeployment> {
    USDT_DEPLOYMENTS.iter().find(|d| d.chain_reference == chain)
}

/// Reverse lookup by jetton minter and CAIP-2 network.
#[cfg(feature = "client")]
#[must_use]
pub fn find_default_tvm_asset(asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
    if network.namespace() != "tvm" {
        return None;
    }
    let chain = TvmChainReference::from_str(network.reference()).ok()?;
    let address: TvmAddress = asset.parse().ok()?;
    usdt_tvm_deployment(chain)
        .filter(|deployment| deployment.address == address)
        .map(|deployment| {
            DefaultAssetInfo::new(
                deployment.address.as_str(),
                u32::from(deployment.decimals),
                "USDT",
            )
        })
}

/// Ergonomic accessors for USDT token deployments on well-known TON chains.
#[derive(Debug, Clone, Copy)]
#[allow(
    clippy::upper_case_acronyms,
    reason = "USDT is a well-known token ticker"
)]
pub struct USDT;

#[allow(
    clippy::doc_markdown,
    clippy::missing_panics_doc,
    clippy::expect_used,
    reason = "static deployment lookups are infallible for built-in data"
)]
impl USDT {
    /// Looks up a USDT deployment by chain reference.
    #[must_use]
    pub fn on(chain: TvmChainReference) -> Option<&'static TvmTokenDeployment> {
        usdt_tvm_deployment(chain)
    }

    /// Returns all known USDT deployments on TON chains.
    #[must_use]
    pub fn all() -> &'static [TvmTokenDeployment] {
        usdt_tvm_deployments()
    }

    /// USDT on TON mainnet (`tvm:-239`).
    #[must_use]
    pub fn tvm() -> &'static TvmTokenDeployment {
        usdt_tvm_deployment(TvmChainReference::MAINNET)
            .expect("built-in USDT deployment for tvm mainnet missing")
    }

    /// USDT on TON testnet (`tvm:-3`).
    #[must_use]
    pub fn tvm_testnet() -> &'static TvmTokenDeployment {
        usdt_tvm_deployment(TvmChainReference::TESTNET)
            .expect("built-in USDT deployment for tvm testnet missing")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn usdt_deployments_resolve() {
        assert_eq!(USDT::tvm().decimals, DEFAULT_TOKEN_DECIMALS);
        assert_eq!(USDT::tvm_testnet().decimals, DEFAULT_TOKEN_DECIMALS);
        assert_eq!(USDT::tvm().address.as_str(), USDT_MAINNET_MINTER);
        assert_eq!(USDT::tvm_testnet().address.as_str(), USDT_TESTNET_MINTER);
        assert_eq!(USDT::all().len(), 2);
        assert_eq!(TVM_NETWORKS.len(), 2);
        assert_eq!(TVM_NETWORKS.first().map(|n| n.reference), Some("-239"));
        assert_eq!(TVM_NETWORKS.get(1).map(|n| n.reference), Some("-3"));
        #[cfg(feature = "client")]
        {
            let network: ChainId = "tvm:-239".parse().unwrap();
            let info = find_default_tvm_asset(USDT_MAINNET_MINTER, &network).unwrap();
            assert_eq!(info.symbol, "USDT");
        }
    }
}
