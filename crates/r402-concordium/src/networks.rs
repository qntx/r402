//! Well-known Concordium networks and default USD-pegged assets.

#[cfg(feature = "client")]
use std::str::FromStr;

#[cfg(feature = "client")]
use r402_client::DefaultAssetInfo;
#[cfg(feature = "client")]
use r402_protocol::ChainId;
use r402_protocol::NetworkInfo;

use crate::chain::{
    CCD_ASSET_IDENTIFIER, CCD_DECIMALS, CONCORDIUM_MAINNET_CAIP_REF, CONCORDIUM_TESTNET_CAIP_REF,
    ConcordiumChainReference, ConcordiumTokenDeployment, USDR_DECIMALS, USDR_TOKEN_ID,
    is_native_ccd,
};

/// Well-known Concordium networks.
pub static CONCORDIUM_NETWORKS: &[NetworkInfo] = &[
    NetworkInfo {
        name: "concordium",
        namespace: "ccd",
        reference: CONCORDIUM_MAINNET_CAIP_REF,
    },
    NetworkInfo {
        name: "concordium-testnet",
        namespace: "ccd",
        reference: CONCORDIUM_TESTNET_CAIP_REF,
    },
];

const USDR_MAINNET: ConcordiumTokenDeployment = ConcordiumTokenDeployment::new(
    ConcordiumChainReference::MAINNET,
    USDR_TOKEN_ID,
    USDR_DECIMALS,
);

const USDR_TESTNET: ConcordiumTokenDeployment = ConcordiumTokenDeployment::new(
    ConcordiumChainReference::TESTNET,
    USDR_TOKEN_ID,
    USDR_DECIMALS,
);

const CCD_MAINNET: ConcordiumTokenDeployment = ConcordiumTokenDeployment::new(
    ConcordiumChainReference::MAINNET,
    CCD_ASSET_IDENTIFIER,
    CCD_DECIMALS,
);

const CCD_TESTNET: ConcordiumTokenDeployment = ConcordiumTokenDeployment::new(
    ConcordiumChainReference::TESTNET,
    CCD_ASSET_IDENTIFIER,
    CCD_DECIMALS,
);

/// Default USD-pegged asset table: index 0 is the `"$0.10"` default (USDR).
#[must_use]
pub fn default_assets(chain: ConcordiumChainReference) -> &'static [ConcordiumTokenDeployment] {
    if chain == ConcordiumChainReference::MAINNET {
        std::slice::from_ref(&USDR_MAINNET)
    } else {
        std::slice::from_ref(&USDR_TESTNET)
    }
}

/// Looks up the default asset for `network` and optional ticker.
///
/// # Errors
///
/// Unknown network or ticker.
pub fn get_default_asset(
    network: &str,
    symbol: Option<&str>,
) -> Result<&'static ConcordiumTokenDeployment, DefaultAssetError> {
    let chain = crate::chain::normalize_concordium_network(network).ok_or_else(|| {
        DefaultAssetError::UnknownNetwork {
            network: network.to_owned(),
        }
    })?;
    let assets = default_assets(chain);
    let Some(first) = assets.first() else {
        return Err(DefaultAssetError::UnknownNetwork {
            network: network.to_owned(),
        });
    };
    let Some(symbol) = symbol else {
        return Ok(first);
    };
    assets
        .iter()
        .find(|entry| entry.asset.eq_ignore_ascii_case(symbol))
        .ok_or_else(|| DefaultAssetError::UnknownTicker {
            symbol: symbol.to_owned(),
            network: network.to_owned(),
        })
}

/// Reverse lookup by PLT token id and CAIP-2 network.
#[must_use]
pub fn find_default_asset(
    asset: &str,
    network: &str,
) -> Option<&'static ConcordiumTokenDeployment> {
    let chain = crate::chain::normalize_concordium_network(network)?;
    default_assets(chain)
        .iter()
        .find(|entry| entry.asset.eq_ignore_ascii_case(asset))
}

/// Reverse lookup used by [`r402_client::SchemeClient`].
#[cfg(feature = "client")]
#[must_use]
pub fn find_default_concordium_asset(asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
    if network.namespace() != "ccd" {
        return None;
    }
    let _ = ConcordiumChainReference::from_str(network.reference()).ok()?;
    find_default_asset(asset, &network.to_string()).map(|deployment| {
        DefaultAssetInfo::new(
            deployment.asset,
            u32::from(deployment.decimals),
            deployment.asset,
        )
    })
}

/// Failure looking up a default asset.
#[derive(Debug, thiserror::Error)]
pub enum DefaultAssetError {
    /// No default-asset table for this CAIP-2 network.
    #[error("No default asset configured for network {network}")]
    UnknownNetwork {
        /// Requested CAIP-2 network.
        network: String,
    },
    /// Ticker is not in the network's default table.
    #[error("No {symbol} default asset configured for network {network}")]
    UnknownTicker {
        /// Requested ticker.
        symbol: String,
        /// Requested CAIP-2 network.
        network: String,
    },
}

/// Ergonomic accessors for USDR.
#[derive(Debug, Clone, Copy)]
#[allow(
    clippy::upper_case_acronyms,
    reason = "USDR is a well-known token ticker"
)]
pub struct USDR;

impl USDR {
    /// USDR on Concordium mainnet.
    #[must_use]
    pub const fn concordium() -> &'static ConcordiumTokenDeployment {
        &USDR_MAINNET
    }

    /// USDR on Concordium testnet.
    #[must_use]
    pub const fn concordium_testnet() -> &'static ConcordiumTokenDeployment {
        &USDR_TESTNET
    }

    /// All USDR rows.
    #[must_use]
    pub const fn all() -> &'static [ConcordiumTokenDeployment] {
        &[USDR_MAINNET, USDR_TESTNET]
    }
}

/// Ergonomic accessors for native CCD.
#[derive(Debug, Clone, Copy)]
#[allow(clippy::upper_case_acronyms, reason = "CCD is the native token ticker")]
pub struct CCD;

impl CCD {
    /// CCD on Concordium mainnet.
    #[must_use]
    pub const fn concordium() -> &'static ConcordiumTokenDeployment {
        &CCD_MAINNET
    }

    /// CCD on Concordium testnet.
    #[must_use]
    pub const fn concordium_testnet() -> &'static ConcordiumTokenDeployment {
        &CCD_TESTNET
    }
}

/// Decimals for a known default asset, or `None` for CCD / unlisted PLTs.
#[must_use]
pub fn asset_decimals(asset: &str, network: &str) -> Option<u8> {
    if is_native_ccd(asset) {
        return None;
    }
    find_default_asset(asset, network).map(|d| d.decimals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::{CONCORDIUM_MAINNET_CAIP2, CONCORDIUM_TESTNET_CAIP2};

    #[test]
    fn usdr_is_network_default() {
        let main = get_default_asset(CONCORDIUM_MAINNET_CAIP2, None).expect("mainnet");
        assert_eq!(main.asset, USDR_TOKEN_ID);
        assert_eq!(main.decimals, USDR_DECIMALS);
        let test = get_default_asset(CONCORDIUM_TESTNET_CAIP2, Some("USDR")).expect("testnet");
        assert_eq!(test.asset, USDR_TOKEN_ID);
    }

    #[test]
    fn find_default_asset_matches_usdr_case_insensitive() {
        assert!(find_default_asset(USDR_TOKEN_ID, CONCORDIUM_MAINNET_CAIP2).is_some());
        assert!(find_default_asset("usdr", CONCORDIUM_TESTNET_CAIP2).is_some());
        assert!(find_default_asset("CCD", CONCORDIUM_MAINNET_CAIP2).is_none());
        assert!(find_default_asset("EURR", CONCORDIUM_TESTNET_CAIP2).is_none());
        assert!(find_default_asset(USDR_TOKEN_ID, "ccd:unknown").is_none());
    }

    #[test]
    fn get_default_asset_rejects_unknown() {
        let err = get_default_asset("ccd:unknown", None).expect_err("unknown net");
        assert!(matches!(err, DefaultAssetError::UnknownNetwork { .. }));
        let ticker_err =
            get_default_asset(CONCORDIUM_TESTNET_CAIP2, Some("EURR")).expect_err("eurr");
        assert!(matches!(
            ticker_err,
            DefaultAssetError::UnknownTicker { .. }
        ));
    }
}
