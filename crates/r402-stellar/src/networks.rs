//! Well-known Stellar network definitions and token deployments.

#[cfg(feature = "client")]
use std::str::FromStr;
use std::sync::LazyLock;

#[cfg(feature = "client")]
use r402_client::DefaultAssetInfo;
#[cfg(feature = "client")]
use r402_protocol::ChainId;
use r402_protocol::NetworkInfo;

use crate::DEFAULT_TOKEN_DECIMALS;
use crate::chain::{StellarAddress, StellarChainReference, StellarTokenDeployment};

/// Well-known Stellar networks with their names and CAIP-2 identifiers.
pub static STELLAR_NETWORKS: &[NetworkInfo] = &[
    NetworkInfo {
        name: "stellar",
        namespace: "stellar",
        reference: "pubnet",
    },
    NetworkInfo {
        name: "stellar-testnet",
        namespace: "stellar",
        reference: "testnet",
    },
];

/// USDC contract on Stellar pubnet.
pub const USDC_PUBNET_ADDRESS: &str = "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75";

/// USDC contract on Stellar testnet.
pub const USDC_TESTNET_ADDRESS: &str = "CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA";

/// Parses a well-known Stellar address, panicking on malformed input.
///
/// Only used for hardcoded constants below; the address strings are fixed
/// and covered by tests, so a panic here indicates a build-time defect.
#[allow(
    clippy::expect_used,
    reason = "hardcoded constant is infallible; validated by tests"
)]
fn well_known(s: &str) -> StellarAddress {
    s.parse().expect("well-known stellar address must be valid")
}

/// Well-known USDC token deployments on Stellar networks.
static USDC_DEPLOYMENTS: LazyLock<Vec<StellarTokenDeployment>> = LazyLock::new(|| {
    vec![
        StellarTokenDeployment::new(
            StellarChainReference::PUBNET,
            well_known(USDC_PUBNET_ADDRESS),
            DEFAULT_TOKEN_DECIMALS,
        ),
        StellarTokenDeployment::new(
            StellarChainReference::TESTNET,
            well_known(USDC_TESTNET_ADDRESS),
            DEFAULT_TOKEN_DECIMALS,
        ),
    ]
});

/// Returns all known USDC deployments on Stellar chains.
#[must_use]
pub fn usdc_stellar_deployments() -> &'static [StellarTokenDeployment] {
    &USDC_DEPLOYMENTS
}

/// Returns the USDC deployment for a specific Stellar chain, if known.
#[must_use]
pub fn usdc_stellar_deployment(
    chain: StellarChainReference,
) -> Option<&'static StellarTokenDeployment> {
    USDC_DEPLOYMENTS.iter().find(|d| d.chain_reference == chain)
}

/// Reverse lookup by SEP-41 contract address and CAIP-2 network.
#[cfg(feature = "client")]
#[must_use]
pub fn find_default_stellar_asset(asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
    if network.namespace() != "stellar" {
        return None;
    }
    let chain = StellarChainReference::from_str(network.reference()).ok()?;
    let address: StellarAddress = asset.parse().ok()?;
    usdc_stellar_deployment(chain)
        .filter(|deployment| deployment.address == address)
        .map(|deployment| {
            DefaultAssetInfo::new(
                deployment.address.to_string(),
                u32::from(deployment.decimals),
                "USDC",
            )
        })
}

/// Ergonomic accessors for USDC token deployments on well-known Stellar chains.
///
/// Combine with [`StellarTokenDeployment::amount`] for a fluent pricing API:
///
/// ```ignore
/// use r402_stellar::{StellarExact, USDC};
///
/// let tag = StellarExact::price_tag(pay_to, USDC::stellar_testnet().amount(10_000_000));
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
    pub fn on(chain: StellarChainReference) -> Option<&'static StellarTokenDeployment> {
        usdc_stellar_deployment(chain)
    }

    /// Returns all known USDC deployments on Stellar chains.
    #[must_use]
    pub fn all() -> &'static [StellarTokenDeployment] {
        usdc_stellar_deployments()
    }

    /// USDC on Stellar pubnet (`stellar:pubnet`).
    #[must_use]
    pub fn stellar() -> &'static StellarTokenDeployment {
        usdc_stellar_deployment(StellarChainReference::PUBNET)
            .expect("built-in USDC deployment for stellar pubnet missing")
    }

    /// USDC on Stellar testnet (`stellar:testnet`).
    #[must_use]
    pub fn stellar_testnet() -> &'static StellarTokenDeployment {
        usdc_stellar_deployment(StellarChainReference::TESTNET)
            .expect("built-in USDC deployment for stellar testnet missing")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usdc_deployments_resolve() {
        assert_eq!(USDC::stellar().decimals, DEFAULT_TOKEN_DECIMALS);
        assert_eq!(USDC::stellar_testnet().decimals, DEFAULT_TOKEN_DECIMALS);
        assert_eq!(USDC::stellar().address.as_str(), USDC_PUBNET_ADDRESS);
        assert_eq!(
            USDC::stellar_testnet().address.as_str(),
            USDC_TESTNET_ADDRESS
        );
        assert!(USDC::stellar().address.is_contract());
        assert_eq!(USDC::all().len(), 2);
        assert_eq!(STELLAR_NETWORKS.len(), 2);
    }
}
