//! USD-pegged default-asset table for EIP-155 networks.

use std::sync::LazyLock;

use alloy_primitives::Address;
use compact_str::CompactString;
use r402_client::DefaultAssetInfo;
use r402_protocol::network::ChainId;

use super::usdc::{USDC_DEPLOYMENTS, USDM_DEPLOYMENTS};
use crate::asset::AssetTransferMethod;
use crate::chain::{Eip155ChainReference, Eip155TokenDeployment, TokenDeploymentEip712};

macro_rules! evm_default_asset {
    ($chain_id:expr, $addr:literal, $dec:literal, $symbol:literal, eip712: $name:literal / $ver:literal) => {
        EvmDefaultAsset {
            chain_reference: Eip155ChainReference::new($chain_id),
            address: alloy_primitives::address!($addr),
            decimals: $dec,
            symbol: $symbol,
            eip712: Some(TokenDeploymentEip712 {
                name: $name.into(),
                version: $ver.into(),
            }),
            asset_transfer_method: None,
            supports_eip2612: false,
        }
    };
    ($chain_id:expr, $addr:literal, $dec:literal, $symbol:literal, eip712: $name:literal / $ver:literal, permit2) => {
        EvmDefaultAsset {
            chain_reference: Eip155ChainReference::new($chain_id),
            address: alloy_primitives::address!($addr),
            decimals: $dec,
            symbol: $symbol,
            eip712: Some(TokenDeploymentEip712 {
                name: $name.into(),
                version: $ver.into(),
            }),
            asset_transfer_method: Some(AssetTransferMethod::Permit2),
            supports_eip2612: false,
        }
    };
    ($chain_id:expr, $addr:literal, $dec:literal, $symbol:literal, eip712: $name:literal / $ver:literal, permit2_eip2612) => {
        EvmDefaultAsset {
            chain_reference: Eip155ChainReference::new($chain_id),
            address: alloy_primitives::address!($addr),
            decimals: $dec,
            symbol: $symbol,
            eip712: Some(TokenDeploymentEip712 {
                name: $name.into(),
                version: $ver.into(),
            }),
            asset_transfer_method: Some(AssetTransferMethod::Permit2),
            supports_eip2612: true,
        }
    };
}

/// USD-pegged default asset for dollar-string prices and spend-cap lookup.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct EvmDefaultAsset {
    /// Chain the token is deployed on.
    pub chain_reference: Eip155ChainReference,
    /// Token contract address.
    pub address: Address,
    /// Token decimal places.
    pub decimals: u8,
    /// Ticker (`"USDC"`, `"USDT0"`, `"mUSD"`, …).
    pub symbol: &'static str,
    /// EIP-712 domain `name` / `version` when the token declares them.
    pub eip712: Option<TokenDeploymentEip712>,
    /// Transfer-method override. `None` means EIP-3009.
    pub asset_transfer_method: Option<AssetTransferMethod>,
    /// Whether a Permit2 token implements EIP-2612 `permit()`.
    pub supports_eip2612: bool,
}

fn usdc_as_default_asset(d: &Eip155TokenDeployment) -> EvmDefaultAsset {
    EvmDefaultAsset {
        chain_reference: d.chain_reference,
        address: d.address,
        decimals: d.decimals,
        symbol: "USDC",
        eip712: d.eip712.clone(),
        asset_transfer_method: None,
        supports_eip2612: false,
    }
}

fn usdm_as_default_asset(d: &Eip155TokenDeployment) -> EvmDefaultAsset {
    EvmDefaultAsset {
        chain_reference: d.chain_reference,
        address: d.address,
        decimals: d.decimals,
        symbol: "MegaUSD",
        eip712: d.eip712.clone(),
        asset_transfer_method: Some(AssetTransferMethod::Permit2),
        supports_eip2612: true,
    }
}

/// Additional USD-pegged default assets (`USDT0`, `mUSD`, `SBC`, `USDC.e`).
static EXTRA_DEFAULT_ASSETS: LazyLock<Vec<EvmDefaultAsset>> = LazyLock::new(|| {
    vec![
        // Stable mainnet USDT0
        evm_default_asset!(988, "0x779Ded0c9e1022225f8E0630b35a9b54bE713736", 6, "USDT0", eip712: "USDT0" / "1"),
        // Stable testnet USDT0
        evm_default_asset!(2201, "0x78Cf24370174180738C5B8E352B6D14c83a6c9A9", 6, "USDT0", eip712: "USDT0" / "1"),
        // Mezo mainnet mUSD
        evm_default_asset!(31612, "0xdD468A1DDc392dcdbEf6db6e34E89AA338F9F186", 18, "mUSD", eip712: "Mezo USD" / "1", permit2_eip2612),
        // Mezo testnet mUSD
        evm_default_asset!(31611, "0x118917a40FAF1CD7a13dB0Ef56C86De7973Ac503", 18, "mUSD", eip712: "Mezo USD" / "1", permit2_eip2612),
        // Radius Network SBC
        evm_default_asset!(723_487, "0x33ad9e4BD16B69B5BFdED37D8B5D9fF9aba014Fb", 6, "SBC", eip712: "Stable Coin" / "1", permit2_eip2612),
        // Radius testnet SBC
        evm_default_asset!(72344, "0x33ad9e4BD16B69B5BFdED37D8B5D9fF9aba014Fb", 6, "SBC", eip712: "Stable Coin" / "1", permit2_eip2612),
        // ADI Chain USDC.e
        evm_default_asset!(36900, "0x9cb8142aEBBcdc60AF7c97Af897A67A8f3CA71C2", 6, "USDC.e", eip712: "USDC.e" / "2"),
        // HPP mainnet USDC.e
        evm_default_asset!(190_415, "0x401eCb1D350407f13ba348573E5630B83638E30D", 6, "USDC.e", eip712: "Bridged USDC" / "2"),
        // HPP Sepolia USDC.e
        evm_default_asset!(181_228, "0x401eCb1D350407f13ba348573E5630B83638E30D", 6, "USDC.e", eip712: "Bridged USDC" / "2"),
        // Igra mainnet USDC
        evm_default_asset!(38833, "0xA5b8BF902b2844dA17d4506cc827F7F1681735E7", 6, "USDC", eip712: "USDC" / "1", permit2),
        // Flare mainnet USD₮0
        evm_default_asset!(14, "0xe7cd86e13AC4309349F30B3435a9d337750fC82D", 6, "USDT0", eip712: "USD\u{20AE}0" / "1"),
    ]
});

static DEFAULT_ASSETS: LazyLock<Vec<EvmDefaultAsset>> = LazyLock::new(|| {
    let mut assets = Vec::with_capacity(
        USDC_DEPLOYMENTS.len() + USDM_DEPLOYMENTS.len() + EXTRA_DEFAULT_ASSETS.len(),
    );
    assets.extend(USDC_DEPLOYMENTS.iter().map(usdc_as_default_asset));
    assets.extend(USDM_DEPLOYMENTS.iter().map(usdm_as_default_asset));
    assets.extend(EXTRA_DEFAULT_ASSETS.iter().cloned());
    assets
});

/// USD-pegged allowlist for dollar-string prices and spend caps, including r402 extra Circle USDC.
#[must_use]
pub fn default_evm_assets() -> &'static [EvmDefaultAsset] {
    &DEFAULT_ASSETS
}

/// Forward lookup: network default when `symbol` is `None`; ticker match is case-insensitive.
#[must_use]
pub fn get_default_evm_asset(
    chain: Eip155ChainReference,
    symbol: Option<&str>,
) -> Option<&'static EvmDefaultAsset> {
    let mut on_chain = DEFAULT_ASSETS
        .iter()
        .filter(|asset| asset.chain_reference == chain);
    match symbol {
        None => on_chain.next(),
        Some(symbol) => on_chain.find(|asset| asset.symbol.eq_ignore_ascii_case(symbol)),
    }
}

/// Reverse lookup by token address and chain.
#[must_use]
pub fn find_default_evm_asset(
    asset: Address,
    chain: Eip155ChainReference,
) -> Option<&'static EvmDefaultAsset> {
    DEFAULT_ASSETS
        .iter()
        .find(|entry| entry.chain_reference == chain && entry.address == asset)
}

impl EvmDefaultAsset {
    /// Client spend-cap view of this row.
    #[must_use]
    pub fn to_default_asset_info(&self) -> DefaultAssetInfo {
        let info = DefaultAssetInfo::new(
            CompactString::from(self.address.to_checksum(None)),
            u32::from(self.decimals),
            self.symbol,
        );
        match self.asset_transfer_method {
            Some(AssetTransferMethod::Permit2) => info.with_asset_transfer_method("permit2"),
            Some(AssetTransferMethod::Eip3009) => info.with_asset_transfer_method("eip3009"),
            None => info,
        }
    }
}

/// Reverse lookup by advertised asset id and CAIP-2 network.
#[must_use]
pub fn find_default_evm_asset_info(asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
    let chain = Eip155ChainReference::try_from(network).ok()?;
    let address = asset.parse().ok()?;
    find_default_evm_asset(address, chain).map(EvmDefaultAsset::to_default_asset_info)
}

#[cfg(test)]
mod tests {
    use alloy_primitives::address;

    use super::super::EVM_NETWORKS;
    use super::*;

    fn chain(id: u64) -> Eip155ChainReference {
        Eip155ChainReference::new(id)
    }

    #[allow(clippy::too_many_arguments, reason = "asserted row fields")]
    fn assert_default_row(
        chain_id: u64,
        asset: Address,
        decimals: u8,
        symbol: &'static str,
        name: &'static str,
        version: &'static str,
        atm: Option<AssetTransferMethod>,
        eip2612: bool,
    ) {
        let found = find_default_evm_asset(asset, chain(chain_id));
        assert_eq!(found.map(|a| a.address), Some(asset), "chain {chain_id}");
        assert_eq!(found.map(|a| a.decimals), Some(decimals));
        assert_eq!(found.map(|a| a.symbol), Some(symbol));
        assert_eq!(found.and_then(|a| a.asset_transfer_method), atm);
        assert_eq!(found.map(|a| a.supports_eip2612), Some(eip2612));
        assert_eq!(
            found.and_then(|a| a.eip712.as_ref().map(|e| e.name.as_str())),
            Some(name)
        );
        assert_eq!(
            found.and_then(|a| a.eip712.as_ref().map(|e| e.version.as_str())),
            Some(version)
        );
        assert_eq!(
            get_default_evm_asset(chain(chain_id), None).map(|a| a.address),
            Some(asset)
        );
        assert_eq!(
            get_default_evm_asset(chain(chain_id), Some(symbol)).map(|a| a.address),
            Some(asset)
        );
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "snapshot of official overlapping defaultAssets rows"
    )]
    fn official_overlapping_caip2_rows_match_name_version_asset() {
        assert_default_row(
            1,
            address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            6,
            "USDC",
            "USD Coin",
            "2",
            None,
            false,
        );
        assert_default_row(
            8453,
            address!("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
            6,
            "USDC",
            "USD Coin",
            "2",
            None,
            false,
        );
        assert_default_row(
            84532,
            address!("0x036CbD53842c5426634e7929541eC2318f3dCF7e"),
            6,
            "USDC",
            "USDC",
            "2",
            None,
            false,
        );
        assert_default_row(
            43114,
            address!("0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E"),
            6,
            "USDC",
            "USD Coin",
            "2",
            None,
            false,
        );
        assert_default_row(
            137,
            address!("0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359"),
            6,
            "USDC",
            "USD Coin",
            "2",
            None,
            false,
        );
        assert_default_row(
            42161,
            address!("0xaf88d065e77c8cC2239327C5EDb3A432268e5831"),
            6,
            "USDC",
            "USD Coin",
            "2",
            None,
            false,
        );
        assert_default_row(
            421_614,
            address!("0x75faf114eafb1BDbe2F0316DF893fd58CE46AA4d"),
            6,
            "USDC",
            "USD Coin",
            "2",
            None,
            false,
        );
        assert_default_row(
            143,
            address!("0x754704Bc059F8C67012fEd69BC8A327a5aafb603"),
            6,
            "USDC",
            "USDC",
            "2",
            None,
            false,
        );
        assert_default_row(
            50,
            address!("0xfA2958CB79b0491CC627c1557F441eF849Ca8eb1"),
            6,
            "USDC",
            "USDC",
            "2",
            None,
            false,
        );
        assert_default_row(
            51,
            address!("0xb5AB69F7bBada22B28e79C8FFAECe55eF1c771D4"),
            6,
            "USDC",
            "USDC",
            "2",
            None,
            false,
        );
        assert_default_row(
            42220,
            address!("0xcebA9300f2b948710d2653dD7B07f33A8B32118C"),
            6,
            "USDC",
            "USDC",
            "2",
            None,
            false,
        );
        assert_default_row(
            11_142_220,
            address!("0x01C5C0122039549AD1493B8220cABEdD739BC44E"),
            6,
            "USDC",
            "USDC",
            "2",
            None,
            false,
        );
        assert_default_row(
            1329,
            address!("0xe15fC38F6D8c56aF07bbCBe3BAf5708A2Bf42392"),
            6,
            "USDC",
            "USDC",
            "2",
            None,
            false,
        );
        assert_default_row(
            1328,
            address!("0x4fCF1784B31630811181f670Aea7A7bEF803eaED"),
            6,
            "USDC",
            "USDC",
            "2",
            None,
            false,
        );
        assert_default_row(
            4326,
            address!("0xFAfDdbb3FC7688494971a79cc65DCa3EF82079E7"),
            18,
            "MegaUSD",
            "MegaUSD",
            "1",
            Some(AssetTransferMethod::Permit2),
            true,
        );
    }

    #[test]
    fn extra_circle_usdc_stays_in_union() {
        let optimism = address!("0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85");
        let found = find_default_evm_asset(optimism, chain(10));
        assert_eq!(found.map(|a| a.symbol), Some("USDC"));
        assert_eq!(found.and_then(|a| a.asset_transfer_method), None);
        assert_eq!(
            get_default_evm_asset(chain(10), None).map(|a| a.address),
            Some(optimism)
        );
    }

    #[test]
    fn megaeth_usdm_is_union_default_not_extra_row() {
        let mega = address!("0xFAfDdbb3FC7688494971a79cc65DCa3EF82079E7");
        let found = find_default_evm_asset(mega, chain(4326));
        assert_eq!(found.map(|a| a.symbol), Some("MegaUSD"));
        assert_eq!(
            found.and_then(|a| a.asset_transfer_method),
            Some(AssetTransferMethod::Permit2)
        );
        assert_eq!(found.map(|a| a.supports_eip2612), Some(true));
        assert_eq!(
            default_evm_assets()
                .iter()
                .filter(|a| a.chain_reference == chain(4326))
                .count(),
            1
        );
        assert!(
            !EXTRA_DEFAULT_ASSETS
                .iter()
                .any(|a| a.chain_reference == chain(4326))
        );
    }

    #[test]
    fn extra_default_assets_match_source_rows() {
        assert_default_row(
            988,
            address!("0x779Ded0c9e1022225f8E0630b35a9b54bE713736"),
            6,
            "USDT0",
            "USDT0",
            "1",
            None,
            false,
        );
        assert_default_row(
            2201,
            address!("0x78Cf24370174180738C5B8E352B6D14c83a6c9A9"),
            6,
            "USDT0",
            "USDT0",
            "1",
            None,
            false,
        );
        assert_default_row(
            31612,
            address!("0xdD468A1DDc392dcdbEf6db6e34E89AA338F9F186"),
            18,
            "mUSD",
            "Mezo USD",
            "1",
            Some(AssetTransferMethod::Permit2),
            true,
        );
        assert_default_row(
            31611,
            address!("0x118917a40FAF1CD7a13dB0Ef56C86De7973Ac503"),
            18,
            "mUSD",
            "Mezo USD",
            "1",
            Some(AssetTransferMethod::Permit2),
            true,
        );
        assert_default_row(
            723_487,
            address!("0x33ad9e4BD16B69B5BFdED37D8B5D9fF9aba014Fb"),
            6,
            "SBC",
            "Stable Coin",
            "1",
            Some(AssetTransferMethod::Permit2),
            true,
        );
        assert_default_row(
            72344,
            address!("0x33ad9e4BD16B69B5BFdED37D8B5D9fF9aba014Fb"),
            6,
            "SBC",
            "Stable Coin",
            "1",
            Some(AssetTransferMethod::Permit2),
            true,
        );
        assert_default_row(
            36900,
            address!("0x9cb8142aEBBcdc60AF7c97Af897A67A8f3CA71C2"),
            6,
            "USDC.e",
            "USDC.e",
            "2",
            None,
            false,
        );
        assert_default_row(
            190_415,
            address!("0x401eCb1D350407f13ba348573E5630B83638E30D"),
            6,
            "USDC.e",
            "Bridged USDC",
            "2",
            None,
            false,
        );
        assert_default_row(
            181_228,
            address!("0x401eCb1D350407f13ba348573E5630B83638E30D"),
            6,
            "USDC.e",
            "Bridged USDC",
            "2",
            None,
            false,
        );
        assert_default_row(
            38833,
            address!("0xA5b8BF902b2844dA17d4506cc827F7F1681735E7"),
            6,
            "USDC",
            "USDC",
            "1",
            Some(AssetTransferMethod::Permit2),
            false,
        );
        assert_default_row(
            14,
            address!("0xe7cd86e13AC4309349F30B3435a9d337750fC82D"),
            6,
            "USDT0",
            "USD\u{20AE}0",
            "1",
            None,
            false,
        );
    }

    #[test]
    fn get_default_asset_symbol_is_case_insensitive() {
        let musd = get_default_evm_asset(chain(31612), Some("musd"));
        assert_eq!(musd.map(|a| a.symbol), Some("mUSD"));
        let usdce = get_default_evm_asset(chain(36900), Some("usdc.e"));
        assert_eq!(usdce.map(|a| a.symbol), Some("USDC.e"));
        assert!(get_default_evm_asset(chain(8453), Some("USDT")).is_none());
        assert!(get_default_evm_asset(chain(999_999), None).is_none());
        assert!(find_default_evm_asset(Address::ZERO, chain(8453)).is_none());
    }

    #[test]
    fn new_networks_are_registered() {
        for (name, reference) in [
            ("stable", "988"),
            ("stable-testnet", "2201"),
            ("mezo", "31612"),
            ("mezo-testnet", "31611"),
            ("radius", "723487"),
            ("radius-testnet", "72344"),
            ("adi", "36900"),
            ("hpp", "190415"),
            ("hpp-sepolia", "181228"),
            ("igra", "38833"),
            ("flare", "14"),
        ] {
            let info = EVM_NETWORKS.iter().find(|n| n.name == name);
            assert_eq!(info.map(|n| n.reference), Some(reference));
            assert_eq!(info.map(|n| n.namespace), Some("eip155"));
        }
    }

    #[test]
    fn union_is_usdc_plus_usdm_plus_extra() {
        assert_eq!(
            default_evm_assets().len(),
            USDC_DEPLOYMENTS.len() + USDM_DEPLOYMENTS.len() + EXTRA_DEFAULT_ASSETS.len()
        );
        assert_eq!(EXTRA_DEFAULT_ASSETS.len(), 11);
        assert_eq!(USDM_DEPLOYMENTS.len(), 1);
    }

    #[test]
    fn find_default_evm_asset_info_matches_lowercase_address() {
        let network = "eip155:8453".parse().unwrap();
        let info =
            find_default_evm_asset_info("0x833589fcd6edb6e08f4c7c32d4f71b54bda02913", &network)
                .unwrap();
        assert_eq!(info.symbol, "USDC");
        assert_eq!(info.decimals, 6);
        assert!(info.asset_transfer_method.is_none());
        let mega = "eip155:4326".parse().unwrap();
        let mega_info =
            find_default_evm_asset_info("0xFAfDdbb3FC7688494971a79cc65DCa3EF82079E7", &mega)
                .unwrap();
        assert_eq!(mega_info.symbol, "MegaUSD");
        assert_eq!(mega_info.asset_transfer_method.as_deref(), Some("permit2"));
    }
}
