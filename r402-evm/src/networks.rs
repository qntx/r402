//! Well-known EVM network definitions and token deployments.
//!
//! This module provides static network metadata and USDC/USDM token deployment
//! information for all supported EIP-155 chains.

use std::sync::LazyLock;

use r402::chain::NetworkInfo;

use crate::chain::{Eip155ChainReference, Eip155TokenDeployment, TokenDeploymentEip712};

macro_rules! evm_networks {
    ($( $name:literal, $ref:literal );* $(;)?) => {
        &[ $( NetworkInfo { name: $name, namespace: "eip155", reference: $ref }, )* ]
    };
}

macro_rules! evm_token_deployment {
    ($chain_id:expr, $addr:literal, $dec:literal, eip712: $name:literal / $ver:literal) => {
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new($chain_id),
            address: alloy_primitives::address!($addr),
            decimals: $dec,
            eip712: Some(TokenDeploymentEip712 {
                name: $name.into(),
                version: $ver.into(),
            }),
        }
    };
    ($chain_id:expr, $addr:literal, $dec:literal) => {
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new($chain_id),
            address: alloy_primitives::address!($addr),
            decimals: $dec,
            eip712: None,
        }
    };
}

macro_rules! token_accessors {
    (
        token: $token:ident, lookup: $lookup_fn:ident;
        $( $(#[doc = $doc:literal])* $method:ident => $chain_id:expr, $label:literal; )*
    ) => {
        $(
            $(#[doc = $doc])*
            #[must_use]
            pub fn $method() -> &'static Eip155TokenDeployment {
                $lookup_fn(Eip155ChainReference::new($chain_id))
                    .expect(concat!("built-in ", stringify!($token), " deployment for ", $label, " missing"))
            }
        )*
    };
}

/// Well-known EVM (EIP-155) networks with their names and CAIP-2 identifiers.
///
/// Source: <https://developers.circle.com/stablecoins/usdc-contract-addresses>
pub static EVM_NETWORKS: &[NetworkInfo] = evm_networks![
    "ethereum",           "1";
    "ethereum-sepolia",   "11155111";
    "base",               "8453";
    "base-sepolia",       "84532";
    "arbitrum",           "42161";
    "arbitrum-sepolia",   "421614";
    "optimism",           "10";
    "optimism-sepolia",   "11155420";
    "polygon",            "137";
    "polygon-amoy",       "80002";
    "avalanche",          "43114";
    "avalanche-fuji",     "43113";
    "celo",               "42220";
    "celo-sepolia",       "11142220";
    "sei",                "1329";
    "sei-testnet",        "1328";
    "sonic",              "146";
    "sonic-blaze",        "57054";
    "unichain",           "130";
    "unichain-sepolia",   "1301";
    "world-chain",        "480";
    "world-chain-sepolia","4801";
    "zksync",             "324";
    "zksync-sepolia",     "300";
    "linea",              "59144";
    "linea-sepolia",      "59141";
    "ink",                "57073";
    "ink-sepolia",        "763373";
    "hyperevm",           "999";
    "hyperevm-testnet",   "998";
    "monad",              "143";
    "monad-testnet",      "10143";
    "plume",              "98866";
    "plume-testnet",      "98867";
    "codex",              "81224";
    "codex-testnet",      "812242";
    "xdc",                "50";
    "xdc-apothem",        "51";
    "xrpl-evm",           "1440000";
    "peaq",               "3338";
    "iotex",              "4689";
    "megaeth",            "4326";
];

/// Well-known USDC token deployments on EVM (EIP-155) networks.
///
/// This is the **single source of truth** for USDC contract addresses, decimal
/// precision, and EIP-712 domain parameters on each supported EVM chain.
///
/// Use [`usdc_evm_deployment()`] for per-chain lookups, or [`usdc_evm_deployments()`]
/// to iterate over all known deployments.
///
/// Source: <https://developers.circle.com/stablecoins/usdc-contract-addresses>
static USDC_DEPLOYMENTS: LazyLock<Vec<Eip155TokenDeployment>> = LazyLock::new(|| {
    vec![
        // Ethereum mainnet — native Circle USDC (FiatTokenV2.1)
        // Verify: https://etherscan.io/token/0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48
        evm_token_deployment!(1, "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", 6, eip712: "USD Coin" / "2"),
        // Ethereum Sepolia — native Circle USDC testnet
        // Verify: https://sepolia.etherscan.io/address/0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238
        evm_token_deployment!(11_155_111, "0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238", 6, eip712: "USDC" / "2"),
        // Base mainnet — native Circle USDC
        // Verify: https://basescan.org/token/0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913
        evm_token_deployment!(8453, "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", 6, eip712: "USD Coin" / "2"),
        // Base Sepolia — native Circle USDC testnet
        // Verify: https://base-sepolia.blockscout.com/address/0x036CbD53842c5426634e7929541eC2318f3dCF7e
        evm_token_deployment!(84532, "0x036CbD53842c5426634e7929541eC2318f3dCF7e", 6, eip712: "USDC" / "2"),
        // Arbitrum One — native Circle USDC
        // Verify: https://arbiscan.io/token/0xaf88d065e77c8cC2239327C5EDb3A432268e5831
        evm_token_deployment!(42161, "0xaf88d065e77c8cC2239327C5EDb3A432268e5831", 6, eip712: "USD Coin" / "2"),
        // Arbitrum Sepolia — native Circle USDC testnet
        // Verify: https://sepolia.arbiscan.io/address/0x75faf114eafb1BDbe2F0316DF893fd58CE46AA4d
        evm_token_deployment!(421_614, "0x75faf114eafb1BDbe2F0316DF893fd58CE46AA4d", 6, eip712: "USDC" / "2"),
        // OP Mainnet — native Circle USDC
        // Verify: https://optimistic.etherscan.io/token/0x0b2c639c533813f4aa9d7837caf62653d097ff85
        evm_token_deployment!(10, "0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85", 6, eip712: "USD Coin" / "2"),
        // OP Sepolia — native Circle USDC testnet
        // Verify: https://sepolia-optimism.etherscan.io/address/0x5fd84259d66Cd46123540766Be93DFE6D43130D7
        evm_token_deployment!(11_155_420, "0x5fd84259d66Cd46123540766Be93DFE6D43130D7", 6, eip712: "USDC" / "2"),
        // Polygon PoS — native Circle USDC (not the old bridged USDC.e at 0x2791...)
        // Verify: https://polygonscan.com/token/0x3c499c542cef5e3811e1192ce70d8cc03d5c3359
        evm_token_deployment!(137, "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359", 6, eip712: "USDC" / "2"),
        // Polygon Amoy — native Circle USDC testnet
        // Verify: https://amoy.polygonscan.com/address/0x41e94eb019c0762f9bfcf9fb1e58725bfb0e7582
        evm_token_deployment!(80002, "0x41E94Eb019C0762f9Bfcf9Fb1E58725BfB0e7582", 6, eip712: "USDC" / "2"),
        // Avalanche C-Chain — native Circle USDC
        // Verify: https://snowtrace.io/token/0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E
        evm_token_deployment!(43114, "0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E", 6, eip712: "USD Coin" / "2"),
        // Avalanche Fuji — native Circle USDC testnet
        // Verify: https://testnet.snowtrace.io/token/0x5425890298aed601595a70ab815c96711a31bc65
        evm_token_deployment!(43113, "0x5425890298aed601595a70AB815c96711a31Bc65", 6, eip712: "USD Coin" / "2"),
        // Celo — native Circle USDC
        // Verify: https://celoscan.io/token/0xcebA9300f2b948710d2653dD7B07f33A8B32118C
        evm_token_deployment!(42220, "0xcebA9300f2b948710d2653dD7B07f33A8B32118C", 6, eip712: "USDC" / "2"),
        // Celo Sepolia — native Circle USDC testnet
        // Verify: https://celo-sepolia.blockscout.com/token/0x01C5C0122039549AD1493B8220cABEdD739BC44E
        evm_token_deployment!(11_142_220, "0x01C5C0122039549AD1493B8220cABEdD739BC44E", 6, eip712: "USDC" / "2"),
        // Sei — native Circle USDC
        // Verify: https://seitrace.com/address/0xe15fC38F6D8c56aF07bbCBe3BAf5708A2Bf42392?chain=pacific-1
        evm_token_deployment!(1329, "0xe15fC38F6D8c56aF07bbCBe3BAf5708A2Bf42392", 6, eip712: "USDC" / "2"),
        // Sei Testnet — native Circle USDC testnet
        // Verify: https://seitrace.com/address/0x4fCF1784B31630811181f670Aea7A7bEF803eaED?chain=atlantic-2
        evm_token_deployment!(1328, "0x4fCF1784B31630811181f670Aea7A7bEF803eaED", 6, eip712: "USDC" / "2"),
        // Sonic — native Circle USDC
        // Verify: https://sonicscan.org/token/0x29219dd400f2bf60e5a23d13be72b486d4038894
        evm_token_deployment!(146, "0x29219dd400f2Bf60E5a23d13Be72B486D4038894", 6, eip712: "USDC" / "2"),
        // Sonic Blaze Testnet — native Circle USDC testnet
        // Verify: https://blaze.soniclabs.com/address/0xA4879Fed32Ecbef99399e5cbC247E533421C4eC6
        evm_token_deployment!(57054, "0xA4879Fed32Ecbef99399e5cbC247E533421C4eC6", 6, eip712: "USDC" / "2"),
        // Unichain — native Circle USDC
        // Verify: https://uniscan.xyz/token/0x078d782b760474a361dda0af3839290b0ef57ad6
        evm_token_deployment!(130, "0x078D782b760474a361dDA0AF3839290b0EF57AD6", 6, eip712: "USDC" / "2"),
        // Unichain Sepolia — native Circle USDC testnet
        // Verify: https://unichain-sepolia.blockscout.com/token/0x31d0220469e10c4E71834a79b1f276d740d3768F
        evm_token_deployment!(1301, "0x31d0220469e10c4E71834a79b1f276d740d3768F", 6, eip712: "USDC" / "2"),
        // World Chain — native Circle USDC
        // Verify: https://worldscan.org/address/0x79A02482A880bCe3F13E09da970dC34dB4cD24D1
        evm_token_deployment!(480, "0x79A02482A880bCe3F13E09da970dC34dB4cD24D1", 6, eip712: "USDC" / "2"),
        // World Chain Sepolia — native Circle USDC testnet
        // Verify: https://sepolia.worldscan.org/address/0x66145f38cBAC35Ca6F1Dfb4914dF98F1614aeA88
        evm_token_deployment!(4801, "0x66145f38cBAC35Ca6F1Dfb4914dF98F1614aeA88", 6, eip712: "USDC" / "2"),
        // ZKsync Era — native Circle USDC
        // Verify: https://explorer.zksync.io/address/0x1d17CBcF0D6D143135aE902365D2E5e2A16538D4
        evm_token_deployment!(324, "0x1d17CBcF0D6D143135aE902365D2E5e2A16538D4", 6, eip712: "USDC" / "2"),
        // ZKsync Era Sepolia — native Circle USDC testnet
        // Verify: https://sepolia.explorer.zksync.io/address/0xAe045DE5638162fa134807Cb558E15A3F5A7F853
        evm_token_deployment!(300, "0xAe045DE5638162fa134807Cb558E15A3F5A7F853", 6, eip712: "USDC" / "2"),
        // Linea — Circle USDC (upgraded from bridged to native via CCTP)
        // Verify: https://lineascan.build/token/0x176211869ca2b568f2a7d4ee941e073a821ee1ff
        evm_token_deployment!(59144, "0x176211869cA2b568f2A7D4EE941E073a821EE1ff", 6, eip712: "USDC" / "2"),
        // Linea Sepolia — Circle USDC testnet
        // Verify: https://sepolia.lineascan.build/address/0xFEce4462D57bD51A6A552365A011b95f0E16d9B7
        evm_token_deployment!(59141, "0xFEce4462D57bD51A6A552365A011b95f0E16d9B7", 6, eip712: "USDC" / "2"),
        // Ink (by Kraken) — native Circle USDC
        // Verify: https://explorer.inkonchain.com/address/0x2D270e6886d130D724215A266106e6832161EAEd
        evm_token_deployment!(57073, "0x2D270e6886d130D724215A266106e6832161EAEd", 6, eip712: "USDC" / "2"),
        // Ink Sepolia — native Circle USDC testnet
        // Verify: https://explorer-sepolia.inkonchain.com/address/0xFabab97dCE620294D2B0b0e46C68964e326300Ac
        evm_token_deployment!(763_373, "0xFabab97dCE620294D2B0b0e46C68964e326300Ac", 6, eip712: "USDC" / "2"),
        // HyperEVM (Hyperliquid) — native Circle USDC
        // Verify: https://hyperscan.com/token/0xb88339CB7199b77E23DB6E890353E22632Ba630f
        evm_token_deployment!(999, "0xb88339CB7199b77E23DB6E890353E22632Ba630f", 6, eip712: "USDC" / "2"),
        // HyperEVM Testnet — native Circle USDC testnet
        // Verify: https://testnet.purrsec.com/address/0x2B3370eE501B4a559b57D449569354196457D8Ab
        evm_token_deployment!(998, "0x2B3370eE501B4a559b57D449569354196457D8Ab", 6, eip712: "USDC" / "2"),
        // Monad — native Circle USDC
        // Verify: https://monadvision.com/token/0x754704Bc059F8C67012fEd69BC8A327a5aafb603
        evm_token_deployment!(143, "0x754704Bc059F8C67012fEd69BC8A327a5aafb603", 6, eip712: "USDC" / "2"),
        // Monad Testnet — native Circle USDC testnet
        // Verify: https://testnet.monadvision.com/token/0x534b2f3A21130d7a60830c2Df862319e593943A3
        evm_token_deployment!(10143, "0x534b2f3A21130d7a60830c2Df862319e593943A3", 6, eip712: "USDC" / "2"),
        // Plume — native Circle USDC
        // Verify: https://explorer.plume.org/address/0x222365EF19F7947e5484218551B56bb3965Aa7aF
        evm_token_deployment!(98866, "0x222365EF19F7947e5484218551B56bb3965Aa7aF", 6, eip712: "USDC" / "2"),
        // Plume Testnet — native Circle USDC testnet
        // Verify: https://testnet-explorer.plume.org/address/0xcB5f30e335672893c7eb944B374c196392C19D18
        evm_token_deployment!(98867, "0xcB5f30e335672893c7eb944B374c196392C19D18", 6, eip712: "USDC" / "2"),
        // Codex — native Circle USDC
        // Verify: https://explorer.codex.xyz/address/0xd996633a415985DBd7D6D12f4A4343E31f5037cf
        evm_token_deployment!(81224, "0xd996633a415985DBd7D6D12f4A4343E31f5037cf", 6, eip712: "USDC" / "2"),
        // Codex Testnet — native Circle USDC testnet
        // Verify: https://explorer.codex-stg.xyz/address/0x6d7f141b6819C2c9CC2f818e6ad549E7Ca090F8f
        evm_token_deployment!(812_242, "0x6d7f141b6819C2c9CC2f818e6ad549E7Ca090F8f", 6, eip712: "USDC" / "2"),
        // XDC Network — native Circle USDC
        // Verify: https://xdcscan.com/address/0xfA2958CB79b0491CC627c1557F441eF849Ca8eb1
        evm_token_deployment!(50, "0xfA2958CB79b0491CC627c1557F441eF849Ca8eb1", 6, eip712: "USDC" / "2"),
        // XDC Apothem Testnet — native Circle USDC testnet
        // Verify: https://testnet.xdcscan.com/address/0xb5AB69F7bBada22B28e79C8FFAECe55eF1c771D4
        evm_token_deployment!(51, "0xb5AB69F7bBada22B28e79C8FFAECe55eF1c771D4", 6, eip712: "USDC" / "2"),
        // XRPL EVM sidechain — community deployment, not on Circle official page
        // EIP-3009 support unverified (eip712: None)
        evm_token_deployment!(1_440_000, "0xDaF4556169c4F3f2231d8ab7BC8772Ddb7D4c84C", 6),
        // Peaq — community deployment, not on Circle official page
        // EIP-3009 support unverified
        evm_token_deployment!(3338, "0xbbA60da06c2c5424f03f7434542280FCAd453d10", 6, eip712: "USDC" / "2"),
        // IoTeX — community deployment, not on Circle official page
        // EIP-3009 support unverified
        evm_token_deployment!(4689, "0xcdf79194c6c285077a58da47641d4dbe51f63542", 6, eip712: "Bridged USDC" / "2"),
    ]
});

/// Well-known USDM token deployments on EVM (EIP-155) networks.
///
/// Use [`usdm_evm_deployment()`] for per-chain lookups.
static USDM_DEPLOYMENTS: LazyLock<Vec<Eip155TokenDeployment>> = LazyLock::new(|| {
    vec![
        // MegaETH — MegaUSD (USDM), the chain's endorsed default stablecoin
        // Matches Go SDK: eip155:4326, name "MegaUSD", version "1", decimals 18
        evm_token_deployment!(4326, "0xFAfDdbb3FC7688494971a79cc65DCa3EF82079E7", 18, eip712: "MegaUSD" / "1"),
    ]
});

/// Returns all known USDC deployments on EVM chains.
#[must_use]
pub fn usdc_evm_deployments() -> &'static [Eip155TokenDeployment] {
    &USDC_DEPLOYMENTS
}

/// Returns the USDC deployment for a specific EVM chain, if known.
#[must_use]
pub fn usdc_evm_deployment(chain: Eip155ChainReference) -> Option<&'static Eip155TokenDeployment> {
    USDC_DEPLOYMENTS.iter().find(|d| d.chain_reference == chain)
}

/// Returns all known USDM deployments on EVM chains.
#[must_use]
pub fn usdm_evm_deployments() -> &'static [Eip155TokenDeployment] {
    &USDM_DEPLOYMENTS
}

/// Returns the USDM deployment for a specific EVM chain, if known.
#[must_use]
pub fn usdm_evm_deployment(chain: Eip155ChainReference) -> Option<&'static Eip155TokenDeployment> {
    USDM_DEPLOYMENTS.iter().find(|d| d.chain_reference == chain)
}

/// Ergonomic accessors for USDC token deployments on well-known EVM chains.
///
/// Provides named methods for each supported chain, returning a static
/// reference to the deployment metadata. Combine with
/// [`Eip155TokenDeployment::amount`] for a fluent pricing API:
///
/// ```ignore
/// use r402_evm::{Eip155Exact, USDC};
///
/// let tag = Eip155Exact::price_tag(pay_to, USDC::base().amount(1_000_000u64), None);
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
    ///
    /// Returns `None` if the chain is not in the built-in deployment table.
    #[must_use]
    pub fn on(chain: Eip155ChainReference) -> Option<&'static Eip155TokenDeployment> {
        usdc_evm_deployment(chain)
    }

    /// Returns all known USDC deployments.
    #[must_use]
    pub fn all() -> &'static [Eip155TokenDeployment] {
        usdc_evm_deployments()
    }

    token_accessors! {
        token: USDC, lookup: usdc_evm_deployment;
        /// USDC on Ethereum mainnet (eip155:1).
        ethereum          => 1,           "Ethereum";
        /// USDC on Ethereum Sepolia testnet (eip155:11155111).
        ethereum_sepolia  => 11_155_111,  "Ethereum Sepolia";
        /// USDC on Base mainnet (eip155:8453).
        base              => 8453,        "Base";
        /// USDC on Base Sepolia testnet (eip155:84532).
        base_sepolia      => 84532,       "Base Sepolia";
        /// USDC on Arbitrum One (eip155:42161).
        arbitrum          => 42161,       "Arbitrum";
        /// USDC on Arbitrum Sepolia testnet (eip155:421614).
        arbitrum_sepolia  => 421_614,     "Arbitrum Sepolia";
        /// USDC on OP Mainnet (eip155:10).
        optimism          => 10,          "Optimism";
        /// USDC on OP Sepolia testnet (eip155:11155420).
        optimism_sepolia  => 11_155_420,  "OP Sepolia";
        /// USDC on Polygon PoS (eip155:137).
        polygon           => 137,         "Polygon";
        /// USDC on Polygon Amoy testnet (eip155:80002).
        polygon_amoy      => 80002,       "Polygon Amoy";
        /// USDC on Avalanche C-Chain (eip155:43114).
        avalanche         => 43114,       "Avalanche";
        /// USDC on Avalanche Fuji testnet (eip155:43113).
        avalanche_fuji    => 43113,       "Avalanche Fuji";
        /// USDC on Celo (eip155:42220).
        celo              => 42220,       "Celo";
        /// USDC on Celo Sepolia testnet (eip155:11142220).
        celo_sepolia      => 11_142_220,  "Celo Sepolia";
        /// USDC on Sonic (eip155:146).
        sonic             => 146,         "Sonic";
        /// USDC on Sonic Blaze testnet (eip155:57054).
        sonic_blaze       => 57054,       "Sonic Blaze";
        /// USDC on Unichain (eip155:130).
        unichain          => 130,         "Unichain";
        /// USDC on Unichain Sepolia testnet (eip155:1301).
        unichain_sepolia  => 1301,        "Unichain Sepolia";
        /// USDC on World Chain (eip155:480).
        world_chain       => 480,         "World Chain";
        /// USDC on World Chain Sepolia testnet (eip155:4801).
        world_chain_sepolia => 4801,      "World Chain Sepolia";
        /// USDC on ZKsync Era (eip155:324).
        zksync            => 324,         "ZKsync";
        /// USDC on ZKsync Era Sepolia testnet (eip155:300).
        zksync_sepolia    => 300,         "ZKsync Sepolia";
        /// USDC on Linea (eip155:59144).
        linea             => 59144,       "Linea";
        /// USDC on Linea Sepolia testnet (eip155:59141).
        linea_sepolia     => 59141,       "Linea Sepolia";
        /// USDC on Ink (eip155:57073).
        ink               => 57073,       "Ink";
        /// USDC on Ink Sepolia testnet (eip155:763373).
        ink_sepolia       => 763_373,     "Ink Sepolia";
        /// USDC on Sei (eip155:1329).
        sei               => 1329,        "Sei";
        /// USDC on Sei testnet (eip155:1328).
        sei_testnet       => 1328,        "Sei testnet";
        /// USDC on HyperEVM (eip155:999).
        hyperevm          => 999,         "HyperEVM";
        /// USDC on HyperEVM testnet (eip155:998).
        hyperevm_testnet  => 998,         "HyperEVM testnet";
        /// USDC on Monad (eip155:143).
        monad             => 143,         "Monad";
        /// USDC on Monad testnet (eip155:10143).
        monad_testnet     => 10143,       "Monad testnet";
        /// USDC on Plume (eip155:98866).
        plume             => 98866,       "Plume";
        /// USDC on Plume testnet (eip155:98867).
        plume_testnet     => 98867,       "Plume testnet";
        /// USDC on Codex (eip155:81224).
        codex             => 81224,       "Codex";
        /// USDC on Codex testnet (eip155:812242).
        codex_testnet     => 812_242,     "Codex testnet";
        /// USDC on XDC Network (eip155:50).
        xdc               => 50,          "XDC";
        /// USDC on XDC Apothem testnet (eip155:51).
        xdc_apothem       => 51,          "XDC Apothem";
    }
}

/// Ergonomic accessors for USDM (`MegaUSD`) token deployments on EVM chains.
///
/// ```ignore
/// use r402_evm::{Eip155Exact, USDM};
///
/// let tag = Eip155Exact::price_tag(pay_to, USDM::megaeth().amount(1_000_000_000_000_000_000u128), None);
/// ```
#[derive(Debug, Clone, Copy)]
#[allow(
    clippy::upper_case_acronyms,
    reason = "USDM is a well-known token ticker"
)]
pub struct USDM;

#[allow(
    clippy::doc_markdown,
    clippy::missing_panics_doc,
    clippy::expect_used,
    reason = "static deployment lookups are infallible for built-in data"
)]
impl USDM {
    /// Looks up a USDM deployment by chain reference.
    #[must_use]
    pub fn on(chain: Eip155ChainReference) -> Option<&'static Eip155TokenDeployment> {
        usdm_evm_deployment(chain)
    }

    /// Returns all known USDM deployments.
    #[must_use]
    pub fn all() -> &'static [Eip155TokenDeployment] {
        usdm_evm_deployments()
    }

    token_accessors! {
        token: USDM, lookup: usdm_evm_deployment;
        /// USDM (MegaUSD) on MegaETH (eip155:4326).
        megaeth => 4326, "MegaETH";
    }
}
