//! Well-known XRPL network definitions and token deployments.

#[cfg(feature = "client")]
use std::str::FromStr;
use std::sync::LazyLock;

use compact_str::CompactString;
#[cfg(feature = "client")]
use r402_client::DefaultAssetInfo;
#[cfg(feature = "client")]
use r402_protocol::network::ChainId;
use r402_protocol::network::NetworkInfo;

use crate::chain::{
    XrplAsset, XrplChainReference, XrplClassicAddress, XrplIssuedCurrency, XrplTokenDeployment,
};
use crate::{
    RLUSD_CURRENCY, RLUSD_DECIMALS, RLUSD_MAINNET_ISSUER, RLUSD_TESTNET_ISSUER, XRP_DECIMALS,
};

/// Well-known XRPL networks with their names and CAIP-2 identifiers.
pub static XRPL_NETWORKS: &[NetworkInfo] = &[
    NetworkInfo {
        name: "xrpl",
        namespace: "xrpl",
        reference: "0",
    },
    NetworkInfo {
        name: "xrpl-testnet",
        namespace: "xrpl",
        reference: "1",
    },
    NetworkInfo {
        name: "xrpl-devnet",
        namespace: "xrpl",
        reference: "2",
    },
];

/// Parses a well-known classic address, panicking on malformed input.
#[allow(
    clippy::expect_used,
    reason = "hardcoded constant is infallible; validated by tests"
)]
fn well_known(s: &str) -> XrplClassicAddress {
    s.parse()
        .expect("well-known xrpl classic address must be valid")
}

fn rlusd(chain: XrplChainReference, issuer: &str) -> XrplTokenDeployment {
    XrplTokenDeployment::new(
        chain,
        XrplAsset::Iou(XrplIssuedCurrency {
            currency: CompactString::from(RLUSD_CURRENCY),
            issuer: well_known(issuer),
        }),
        RLUSD_DECIMALS,
    )
}

/// Native XRP on well-known networks.
static XRP_DEPLOYMENTS: LazyLock<Vec<XrplTokenDeployment>> = LazyLock::new(|| {
    vec![
        XrplTokenDeployment::new(XrplChainReference::MAINNET, XrplAsset::Xrp, XRP_DECIMALS),
        XrplTokenDeployment::new(XrplChainReference::TESTNET, XrplAsset::Xrp, XRP_DECIMALS),
        XrplTokenDeployment::new(XrplChainReference::DEVNET, XrplAsset::Xrp, XRP_DECIMALS),
    ]
});

/// RLUSD issued-currency deployments.
static RLUSD_DEPLOYMENTS: LazyLock<Vec<XrplTokenDeployment>> = LazyLock::new(|| {
    vec![
        rlusd(XrplChainReference::MAINNET, RLUSD_MAINNET_ISSUER),
        rlusd(XrplChainReference::TESTNET, RLUSD_TESTNET_ISSUER),
    ]
});

/// Returns all known native XRP deployments.
#[must_use]
pub fn xrp_deployments() -> &'static [XrplTokenDeployment] {
    &XRP_DEPLOYMENTS
}

/// Returns the native XRP deployment for a specific XRPL chain.
#[must_use]
pub fn xrp_deployment(chain: XrplChainReference) -> Option<&'static XrplTokenDeployment> {
    XRP_DEPLOYMENTS.iter().find(|d| d.chain_reference == chain)
}

/// Returns all known RLUSD deployments.
#[must_use]
pub fn rlusd_deployments() -> &'static [XrplTokenDeployment] {
    &RLUSD_DEPLOYMENTS
}

/// Returns the RLUSD deployment for a specific XRPL chain, if known.
#[must_use]
pub fn rlusd_deployment(chain: XrplChainReference) -> Option<&'static XrplTokenDeployment> {
    RLUSD_DEPLOYMENTS
        .iter()
        .find(|d| d.chain_reference == chain)
}

/// Reverse lookup by RLUSD currency hex and CAIP-2 network.
#[cfg(feature = "client")]
#[must_use]
pub fn find_default_xrpl_asset(asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
    if network.namespace() != "xrpl" {
        return None;
    }
    let chain = XrplChainReference::from_str(network.reference()).ok()?;
    rlusd_deployment(chain)
        .filter(|deployment| deployment.asset.as_str().eq_ignore_ascii_case(asset))
        .map(|deployment| {
            DefaultAssetInfo::new(
                deployment.asset.as_str(),
                u32::from(deployment.decimals),
                "RLUSD",
            )
        })
}

/// Ergonomic accessors for native XRP.
#[derive(Debug, Clone, Copy)]
#[allow(
    clippy::upper_case_acronyms,
    reason = "XRP is a well-known token ticker"
)]
pub struct XRP;

#[allow(
    clippy::missing_panics_doc,
    clippy::expect_used,
    reason = "static deployment lookups are infallible for built-in data"
)]
impl XRP {
    /// Native XRP on XRPL mainnet (`xrpl:0`).
    #[must_use]
    pub fn mainnet() -> &'static XrplTokenDeployment {
        xrp_deployment(XrplChainReference::MAINNET)
            .expect("built-in XRP deployment for xrpl mainnet missing")
    }

    /// Native XRP on XRPL testnet (`xrpl:1`).
    #[must_use]
    pub fn testnet() -> &'static XrplTokenDeployment {
        xrp_deployment(XrplChainReference::TESTNET)
            .expect("built-in XRP deployment for xrpl testnet missing")
    }

    /// Native XRP on XRPL devnet (`xrpl:2`).
    #[must_use]
    pub fn devnet() -> &'static XrplTokenDeployment {
        xrp_deployment(XrplChainReference::DEVNET)
            .expect("built-in XRP deployment for xrpl devnet missing")
    }
}

/// Ergonomic accessors for RLUSD issued currency.
#[derive(Debug, Clone, Copy)]
#[allow(
    clippy::upper_case_acronyms,
    reason = "RLUSD is a well-known token ticker"
)]
pub struct RLUSD;

#[allow(
    clippy::missing_panics_doc,
    clippy::expect_used,
    reason = "static deployment lookups are infallible for built-in data"
)]
impl RLUSD {
    /// RLUSD on XRPL mainnet (`xrpl:0`).
    #[must_use]
    pub fn mainnet() -> &'static XrplTokenDeployment {
        rlusd_deployment(XrplChainReference::MAINNET)
            .expect("built-in RLUSD deployment for xrpl mainnet missing")
    }

    /// RLUSD on XRPL testnet (`xrpl:1`).
    #[must_use]
    pub fn testnet() -> &'static XrplTokenDeployment {
        rlusd_deployment(XrplChainReference::TESTNET)
            .expect("built-in RLUSD deployment for xrpl testnet missing")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn deployments_resolve() {
        assert_eq!(XRP::mainnet().decimals, XRP_DECIMALS);
        assert_eq!(RLUSD::mainnet().decimals, RLUSD_DECIMALS);
        assert_eq!(RLUSD::testnet().decimals, RLUSD_DECIMALS);
        assert_eq!(XRPL_NETWORKS.len(), 3);
        match &RLUSD::mainnet().asset {
            XrplAsset::Iou(iou) => {
                assert_eq!(iou.currency, RLUSD_CURRENCY);
                assert_eq!(iou.issuer.as_str(), RLUSD_MAINNET_ISSUER);
            }
            XrplAsset::Xrp => panic!("expected IOU"),
        }
        #[cfg(feature = "client")]
        {
            let network: ChainId = "xrpl:0".parse().unwrap();
            let info = find_default_xrpl_asset(RLUSD_CURRENCY, &network).unwrap();
            assert_eq!(info.symbol, "RLUSD");
            assert_eq!(info.decimals, u32::from(RLUSD_DECIMALS));
            assert!(find_default_xrpl_asset("XRP", &network).is_none());
        }
    }
}
