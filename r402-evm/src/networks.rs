//! Well-known EVM network definitions and token deployments.
//!
//! This module provides static network metadata and USDC/USDM token deployment
//! information for all supported EIP-155 chains.

use std::sync::LazyLock;

use r402::networks::NetworkInfo;

use crate::chain::{Eip155ChainReference, Eip155TokenDeployment, TokenDeploymentEip712};

/// Well-known EVM (EIP-155) networks with their names and CAIP-2 identifiers.
///
/// Source: <https://developers.circle.com/stablecoins/usdc-contract-addresses>
pub static EVM_NETWORKS: &[NetworkInfo] = &[
    NetworkInfo {
        name: "ethereum",
        namespace: "eip155",
        reference: "1",
    },
    NetworkInfo {
        name: "ethereum-sepolia",
        namespace: "eip155",
        reference: "11155111",
    },
    NetworkInfo {
        name: "base",
        namespace: "eip155",
        reference: "8453",
    },
    NetworkInfo {
        name: "base-sepolia",
        namespace: "eip155",
        reference: "84532",
    },
    NetworkInfo {
        name: "arbitrum",
        namespace: "eip155",
        reference: "42161",
    },
    NetworkInfo {
        name: "arbitrum-sepolia",
        namespace: "eip155",
        reference: "421614",
    },
    NetworkInfo {
        name: "optimism",
        namespace: "eip155",
        reference: "10",
    },
    NetworkInfo {
        name: "optimism-sepolia",
        namespace: "eip155",
        reference: "11155420",
    },
    NetworkInfo {
        name: "polygon",
        namespace: "eip155",
        reference: "137",
    },
    NetworkInfo {
        name: "polygon-amoy",
        namespace: "eip155",
        reference: "80002",
    },
    NetworkInfo {
        name: "avalanche",
        namespace: "eip155",
        reference: "43114",
    },
    NetworkInfo {
        name: "avalanche-fuji",
        namespace: "eip155",
        reference: "43113",
    },
    NetworkInfo {
        name: "celo",
        namespace: "eip155",
        reference: "42220",
    },
    NetworkInfo {
        name: "celo-sepolia",
        namespace: "eip155",
        reference: "11142220",
    },
    NetworkInfo {
        name: "sei",
        namespace: "eip155",
        reference: "1329",
    },
    NetworkInfo {
        name: "sei-testnet",
        namespace: "eip155",
        reference: "1328",
    },
    NetworkInfo {
        name: "sonic",
        namespace: "eip155",
        reference: "146",
    },
    NetworkInfo {
        name: "sonic-blaze",
        namespace: "eip155",
        reference: "57054",
    },
    NetworkInfo {
        name: "unichain",
        namespace: "eip155",
        reference: "130",
    },
    NetworkInfo {
        name: "unichain-sepolia",
        namespace: "eip155",
        reference: "1301",
    },
    NetworkInfo {
        name: "world-chain",
        namespace: "eip155",
        reference: "480",
    },
    NetworkInfo {
        name: "world-chain-sepolia",
        namespace: "eip155",
        reference: "4801",
    },
    NetworkInfo {
        name: "zksync",
        namespace: "eip155",
        reference: "324",
    },
    NetworkInfo {
        name: "zksync-sepolia",
        namespace: "eip155",
        reference: "300",
    },
    NetworkInfo {
        name: "linea",
        namespace: "eip155",
        reference: "59144",
    },
    NetworkInfo {
        name: "linea-sepolia",
        namespace: "eip155",
        reference: "59141",
    },
    NetworkInfo {
        name: "ink",
        namespace: "eip155",
        reference: "57073",
    },
    NetworkInfo {
        name: "ink-sepolia",
        namespace: "eip155",
        reference: "763373",
    },
    NetworkInfo {
        name: "hyperevm",
        namespace: "eip155",
        reference: "999",
    },
    NetworkInfo {
        name: "hyperevm-testnet",
        namespace: "eip155",
        reference: "998",
    },
    NetworkInfo {
        name: "monad",
        namespace: "eip155",
        reference: "143",
    },
    NetworkInfo {
        name: "monad-testnet",
        namespace: "eip155",
        reference: "10143",
    },
    NetworkInfo {
        name: "plume",
        namespace: "eip155",
        reference: "98866",
    },
    NetworkInfo {
        name: "plume-testnet",
        namespace: "eip155",
        reference: "98867",
    },
    NetworkInfo {
        name: "codex",
        namespace: "eip155",
        reference: "81224",
    },
    NetworkInfo {
        name: "codex-testnet",
        namespace: "eip155",
        reference: "812242",
    },
    NetworkInfo {
        name: "xdc",
        namespace: "eip155",
        reference: "50",
    },
    NetworkInfo {
        name: "xdc-apothem",
        namespace: "eip155",
        reference: "51",
    },
    NetworkInfo {
        name: "xrpl-evm",
        namespace: "eip155",
        reference: "1440000",
    },
    NetworkInfo {
        name: "peaq",
        namespace: "eip155",
        reference: "3338",
    },
    NetworkInfo {
        name: "iotex",
        namespace: "eip155",
        reference: "4689",
    },
    NetworkInfo {
        name: "megaeth",
        namespace: "eip155",
        reference: "4326",
    },
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
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(1),
            address: alloy_primitives::address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USD Coin".into(),
                version: "2".into(),
            }),
        },
        // Ethereum Sepolia — native Circle USDC testnet
        // Verify: https://sepolia.etherscan.io/address/0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(11_155_111),
            address: alloy_primitives::address!("0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Base mainnet — native Circle USDC
        // Verify: https://basescan.org/token/0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(8453),
            address: alloy_primitives::address!("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USD Coin".into(),
                version: "2".into(),
            }),
        },
        // Base Sepolia — native Circle USDC testnet
        // Verify: https://base-sepolia.blockscout.com/address/0x036CbD53842c5426634e7929541eC2318f3dCF7e
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(84532),
            address: alloy_primitives::address!("0x036CbD53842c5426634e7929541eC2318f3dCF7e"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Arbitrum One — native Circle USDC
        // Verify: https://arbiscan.io/token/0xaf88d065e77c8cC2239327C5EDb3A432268e5831
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(42161),
            address: alloy_primitives::address!("0xaf88d065e77c8cC2239327C5EDb3A432268e5831"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USD Coin".into(),
                version: "2".into(),
            }),
        },
        // Arbitrum Sepolia — native Circle USDC testnet
        // Verify: https://sepolia.arbiscan.io/address/0x75faf114eafb1BDbe2F0316DF893fd58CE46AA4d
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(421_614),
            address: alloy_primitives::address!("0x75faf114eafb1BDbe2F0316DF893fd58CE46AA4d"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // OP Mainnet — native Circle USDC
        // Verify: https://optimistic.etherscan.io/token/0x0b2c639c533813f4aa9d7837caf62653d097ff85
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(10),
            address: alloy_primitives::address!("0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USD Coin".into(),
                version: "2".into(),
            }),
        },
        // OP Sepolia — native Circle USDC testnet
        // Verify: https://sepolia-optimism.etherscan.io/address/0x5fd84259d66Cd46123540766Be93DFE6D43130D7
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(11_155_420),
            address: alloy_primitives::address!("0x5fd84259d66Cd46123540766Be93DFE6D43130D7"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Polygon PoS — native Circle USDC (not the old bridged USDC.e at 0x2791...)
        // Verify: https://polygonscan.com/token/0x3c499c542cef5e3811e1192ce70d8cc03d5c3359
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(137),
            address: alloy_primitives::address!("0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Polygon Amoy — native Circle USDC testnet
        // Verify: https://amoy.polygonscan.com/address/0x41e94eb019c0762f9bfcf9fb1e58725bfb0e7582
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(80002),
            address: alloy_primitives::address!("0x41E94Eb019C0762f9Bfcf9Fb1E58725BfB0e7582"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Avalanche C-Chain — native Circle USDC
        // Verify: https://snowtrace.io/token/0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(43114),
            address: alloy_primitives::address!("0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USD Coin".into(),
                version: "2".into(),
            }),
        },
        // Avalanche Fuji — native Circle USDC testnet
        // Verify: https://testnet.snowtrace.io/token/0x5425890298aed601595a70ab815c96711a31bc65
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(43113),
            address: alloy_primitives::address!("0x5425890298aed601595a70AB815c96711a31Bc65"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USD Coin".into(),
                version: "2".into(),
            }),
        },
        // Celo — native Circle USDC
        // Verify: https://celoscan.io/token/0xcebA9300f2b948710d2653dD7B07f33A8B32118C
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(42220),
            address: alloy_primitives::address!("0xcebA9300f2b948710d2653dD7B07f33A8B32118C"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Celo Sepolia — native Circle USDC testnet
        // Verify: https://celo-sepolia.blockscout.com/token/0x01C5C0122039549AD1493B8220cABEdD739BC44E
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(11_142_220),
            address: alloy_primitives::address!("0x01C5C0122039549AD1493B8220cABEdD739BC44E"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Sei — native Circle USDC
        // Verify: https://seitrace.com/address/0xe15fC38F6D8c56aF07bbCBe3BAf5708A2Bf42392?chain=pacific-1
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(1329),
            address: alloy_primitives::address!("0xe15fC38F6D8c56aF07bbCBe3BAf5708A2Bf42392"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Sei Testnet — native Circle USDC testnet
        // Verify: https://seitrace.com/address/0x4fCF1784B31630811181f670Aea7A7bEF803eaED?chain=atlantic-2
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(1328),
            address: alloy_primitives::address!("0x4fCF1784B31630811181f670Aea7A7bEF803eaED"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Sonic — native Circle USDC
        // Verify: https://sonicscan.org/token/0x29219dd400f2bf60e5a23d13be72b486d4038894
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(146),
            address: alloy_primitives::address!("0x29219dd400f2Bf60E5a23d13Be72B486D4038894"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Sonic Blaze Testnet — native Circle USDC testnet
        // Verify: https://blaze.soniclabs.com/address/0xA4879Fed32Ecbef99399e5cbC247E533421C4eC6
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(57054),
            address: alloy_primitives::address!("0xA4879Fed32Ecbef99399e5cbC247E533421C4eC6"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Unichain — native Circle USDC
        // Verify: https://uniscan.xyz/token/0x078d782b760474a361dda0af3839290b0ef57ad6
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(130),
            address: alloy_primitives::address!("0x078D782b760474a361dDA0AF3839290b0EF57AD6"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Unichain Sepolia — native Circle USDC testnet
        // Verify: https://unichain-sepolia.blockscout.com/token/0x31d0220469e10c4E71834a79b1f276d740d3768F
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(1301),
            address: alloy_primitives::address!("0x31d0220469e10c4E71834a79b1f276d740d3768F"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // World Chain — native Circle USDC
        // Verify: https://worldscan.org/address/0x79A02482A880bCe3F13E09da970dC34dB4cD24D1
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(480),
            address: alloy_primitives::address!("0x79A02482A880bCe3F13E09da970dC34dB4cD24D1"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // World Chain Sepolia — native Circle USDC testnet
        // Verify: https://sepolia.worldscan.org/address/0x66145f38cBAC35Ca6F1Dfb4914dF98F1614aeA88
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(4801),
            address: alloy_primitives::address!("0x66145f38cBAC35Ca6F1Dfb4914dF98F1614aeA88"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // ZKsync Era — native Circle USDC
        // Verify: https://explorer.zksync.io/address/0x1d17CBcF0D6D143135aE902365D2E5e2A16538D4
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(324),
            address: alloy_primitives::address!("0x1d17CBcF0D6D143135aE902365D2E5e2A16538D4"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // ZKsync Era Sepolia — native Circle USDC testnet
        // Verify: https://sepolia.explorer.zksync.io/address/0xAe045DE5638162fa134807Cb558E15A3F5A7F853
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(300),
            address: alloy_primitives::address!("0xAe045DE5638162fa134807Cb558E15A3F5A7F853"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Linea — Circle USDC (upgraded from bridged to native via CCTP)
        // Verify: https://lineascan.build/token/0x176211869ca2b568f2a7d4ee941e073a821ee1ff
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(59144),
            address: alloy_primitives::address!("0x176211869cA2b568f2A7D4EE941E073a821EE1ff"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Linea Sepolia — Circle USDC testnet
        // Verify: https://sepolia.lineascan.build/address/0xFEce4462D57bD51A6A552365A011b95f0E16d9B7
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(59141),
            address: alloy_primitives::address!("0xFEce4462D57bD51A6A552365A011b95f0E16d9B7"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Ink (by Kraken) — native Circle USDC
        // Verify: https://explorer.inkonchain.com/address/0x2D270e6886d130D724215A266106e6832161EAEd
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(57073),
            address: alloy_primitives::address!("0x2D270e6886d130D724215A266106e6832161EAEd"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Ink Sepolia — native Circle USDC testnet
        // Verify: https://explorer-sepolia.inkonchain.com/address/0xFabab97dCE620294D2B0b0e46C68964e326300Ac
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(763_373),
            address: alloy_primitives::address!("0xFabab97dCE620294D2B0b0e46C68964e326300Ac"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // HyperEVM (Hyperliquid) — native Circle USDC
        // Verify: https://hyperscan.com/token/0xb88339CB7199b77E23DB6E890353E22632Ba630f
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(999),
            address: alloy_primitives::address!("0xb88339CB7199b77E23DB6E890353E22632Ba630f"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // HyperEVM Testnet — native Circle USDC testnet
        // Verify: https://testnet.purrsec.com/address/0x2B3370eE501B4a559b57D449569354196457D8Ab
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(998),
            address: alloy_primitives::address!("0x2B3370eE501B4a559b57D449569354196457D8Ab"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Monad — native Circle USDC
        // Verify: https://monadvision.com/token/0x754704Bc059F8C67012fEd69BC8A327a5aafb603
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(143),
            address: alloy_primitives::address!("0x754704Bc059F8C67012fEd69BC8A327a5aafb603"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Monad Testnet — native Circle USDC testnet
        // Verify: https://testnet.monadvision.com/token/0x534b2f3A21130d7a60830c2Df862319e593943A3
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(10143),
            address: alloy_primitives::address!("0x534b2f3A21130d7a60830c2Df862319e593943A3"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Plume — native Circle USDC
        // Verify: https://explorer.plume.org/address/0x222365EF19F7947e5484218551B56bb3965Aa7aF
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(98866),
            address: alloy_primitives::address!("0x222365EF19F7947e5484218551B56bb3965Aa7aF"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Plume Testnet — native Circle USDC testnet
        // Verify: https://testnet-explorer.plume.org/address/0xcB5f30e335672893c7eb944B374c196392C19D18
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(98867),
            address: alloy_primitives::address!("0xcB5f30e335672893c7eb944B374c196392C19D18"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Codex — native Circle USDC
        // Verify: https://explorer.codex.xyz/address/0xd996633a415985DBd7D6D12f4A4343E31f5037cf
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(81224),
            address: alloy_primitives::address!("0xd996633a415985DBd7D6D12f4A4343E31f5037cf"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // Codex Testnet — native Circle USDC testnet
        // Verify: https://explorer.codex-stg.xyz/address/0x6d7f141b6819C2c9CC2f818e6ad549E7Ca090F8f
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(812_242),
            address: alloy_primitives::address!("0x6d7f141b6819C2c9CC2f818e6ad549E7Ca090F8f"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // XDC Network — native Circle USDC
        // Verify: https://xdcscan.com/address/0xfA2958CB79b0491CC627c1557F441eF849Ca8eb1
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(50),
            address: alloy_primitives::address!("0xfA2958CB79b0491CC627c1557F441eF849Ca8eb1"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // XDC Apothem Testnet — native Circle USDC testnet
        // Verify: https://testnet.xdcscan.com/address/0xb5AB69F7bBada22B28e79C8FFAECe55eF1c771D4
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(51),
            address: alloy_primitives::address!("0xb5AB69F7bBada22B28e79C8FFAECe55eF1c771D4"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // XRPL EVM sidechain — community deployment, not on Circle official page
        // EIP-3009 support unverified (eip712: None)
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(1_440_000),
            address: alloy_primitives::address!("0xDaF4556169c4F3f2231d8ab7BC8772Ddb7D4c84C"),
            decimals: 6,
            eip712: None,
        },
        // Peaq — community deployment, not on Circle official page
        // EIP-3009 support unverified
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(3338),
            address: alloy_primitives::address!("0xbbA60da06c2c5424f03f7434542280FCAd453d10"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        },
        // IoTeX — community deployment, not on Circle official page
        // EIP-3009 support unverified
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(4689),
            address: alloy_primitives::address!("0xcdf79194c6c285077a58da47641d4dbe51f63542"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "Bridged USDC".into(),
                version: "2".into(),
            }),
        },
    ]
});

/// Well-known USDM token deployments on EVM (EIP-155) networks.
///
/// Use [`usdm_evm_deployment()`] for per-chain lookups.
static USDM_DEPLOYMENTS: LazyLock<Vec<Eip155TokenDeployment>> = LazyLock::new(|| {
    vec![
        // MegaETH — MegaUSD (USDM), the chain's endorsed default stablecoin
        // Matches Go SDK: eip155:4326, name "MegaUSD", version "1", decimals 18
        Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(4326),
            address: alloy_primitives::address!("0xFAfDdbb3FC7688494971a79cc65DCa3EF82079E7"),
            decimals: 18,
            eip712: Some(TokenDeploymentEip712 {
                name: "MegaUSD".into(),
                version: "1".into(),
            }),
        },
    ]
});

/// Returns all known USDC deployments on EVM chains.
#[must_use]
pub fn usdc_evm_deployments() -> &'static [Eip155TokenDeployment] {
    &USDC_DEPLOYMENTS
}

/// Returns the USDC deployment for a specific EVM chain, if known.
#[must_use]
pub fn usdc_evm_deployment(chain: &Eip155ChainReference) -> Option<&'static Eip155TokenDeployment> {
    USDC_DEPLOYMENTS
        .iter()
        .find(|d| d.chain_reference == *chain)
}

/// Returns all known USDM deployments on EVM chains.
#[must_use]
pub fn usdm_evm_deployments() -> &'static [Eip155TokenDeployment] {
    &USDM_DEPLOYMENTS
}

/// Returns the USDM deployment for a specific EVM chain, if known.
#[must_use]
pub fn usdm_evm_deployment(chain: &Eip155ChainReference) -> Option<&'static Eip155TokenDeployment> {
    USDM_DEPLOYMENTS
        .iter()
        .find(|d| d.chain_reference == *chain)
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
pub struct USDC;

#[allow(clippy::doc_markdown, clippy::missing_panics_doc)]
impl USDC {
    /// Looks up a USDC deployment by chain reference.
    ///
    /// Returns `None` if the chain is not in the built-in deployment table.
    #[must_use]
    pub fn on(chain: &Eip155ChainReference) -> Option<&'static Eip155TokenDeployment> {
        usdc_evm_deployment(chain)
    }

    /// Returns all known USDC deployments.
    #[must_use]
    pub fn all() -> &'static [Eip155TokenDeployment] {
        usdc_evm_deployments()
    }

    /// USDC on Ethereum mainnet (eip155:1).
    #[must_use]
    pub fn ethereum() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(1))
            .expect("built-in USDC deployment for Ethereum missing")
    }

    /// USDC on Ethereum Sepolia testnet (eip155:11155111).
    #[must_use]
    pub fn ethereum_sepolia() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(11_155_111))
            .expect("built-in USDC deployment for Ethereum Sepolia missing")
    }

    /// USDC on Base mainnet (eip155:8453).
    #[must_use]
    pub fn base() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(8453))
            .expect("built-in USDC deployment for Base missing")
    }

    /// USDC on Base Sepolia testnet (eip155:84532).
    #[must_use]
    pub fn base_sepolia() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(84532))
            .expect("built-in USDC deployment for Base Sepolia missing")
    }

    /// USDC on Arbitrum One (eip155:42161).
    #[must_use]
    pub fn arbitrum() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(42161))
            .expect("built-in USDC deployment for Arbitrum missing")
    }

    /// USDC on Arbitrum Sepolia testnet (eip155:421614).
    #[must_use]
    pub fn arbitrum_sepolia() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(421_614))
            .expect("built-in USDC deployment for Arbitrum Sepolia missing")
    }

    /// USDC on OP Mainnet (eip155:10).
    #[must_use]
    pub fn optimism() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(10))
            .expect("built-in USDC deployment for Optimism missing")
    }

    /// USDC on OP Sepolia testnet (eip155:11155420).
    #[must_use]
    pub fn optimism_sepolia() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(11_155_420))
            .expect("built-in USDC deployment for OP Sepolia missing")
    }

    /// USDC on Polygon PoS (eip155:137).
    #[must_use]
    pub fn polygon() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(137))
            .expect("built-in USDC deployment for Polygon missing")
    }

    /// USDC on Polygon Amoy testnet (eip155:80002).
    #[must_use]
    pub fn polygon_amoy() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(80002))
            .expect("built-in USDC deployment for Polygon Amoy missing")
    }

    /// USDC on Avalanche C-Chain (eip155:43114).
    #[must_use]
    pub fn avalanche() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(43114))
            .expect("built-in USDC deployment for Avalanche missing")
    }

    /// USDC on Avalanche Fuji testnet (eip155:43113).
    #[must_use]
    pub fn avalanche_fuji() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(43113))
            .expect("built-in USDC deployment for Avalanche Fuji missing")
    }

    /// USDC on Celo (eip155:42220).
    #[must_use]
    pub fn celo() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(42220))
            .expect("built-in USDC deployment for Celo missing")
    }

    /// USDC on Celo Sepolia testnet (eip155:11142220).
    #[must_use]
    pub fn celo_sepolia() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(11_142_220))
            .expect("built-in USDC deployment for Celo Sepolia missing")
    }

    /// USDC on Sonic (eip155:146).
    #[must_use]
    pub fn sonic() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(146))
            .expect("built-in USDC deployment for Sonic missing")
    }

    /// USDC on Sonic Blaze testnet (eip155:57054).
    #[must_use]
    pub fn sonic_blaze() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(57054))
            .expect("built-in USDC deployment for Sonic Blaze missing")
    }

    /// USDC on Unichain (eip155:130).
    #[must_use]
    pub fn unichain() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(130))
            .expect("built-in USDC deployment for Unichain missing")
    }

    /// USDC on Unichain Sepolia testnet (eip155:1301).
    #[must_use]
    pub fn unichain_sepolia() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(1301))
            .expect("built-in USDC deployment for Unichain Sepolia missing")
    }

    /// USDC on World Chain (eip155:480).
    #[must_use]
    pub fn world_chain() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(480))
            .expect("built-in USDC deployment for World Chain missing")
    }

    /// USDC on World Chain Sepolia testnet (eip155:4801).
    #[must_use]
    pub fn world_chain_sepolia() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(4801))
            .expect("built-in USDC deployment for World Chain Sepolia missing")
    }

    /// USDC on ZKsync Era (eip155:324).
    #[must_use]
    pub fn zksync() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(324))
            .expect("built-in USDC deployment for ZKsync missing")
    }

    /// USDC on ZKsync Era Sepolia testnet (eip155:300).
    #[must_use]
    pub fn zksync_sepolia() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(300))
            .expect("built-in USDC deployment for ZKsync Sepolia missing")
    }

    /// USDC on Linea (eip155:59144).
    #[must_use]
    pub fn linea() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(59144))
            .expect("built-in USDC deployment for Linea missing")
    }

    /// USDC on Linea Sepolia testnet (eip155:59141).
    #[must_use]
    pub fn linea_sepolia() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(59141))
            .expect("built-in USDC deployment for Linea Sepolia missing")
    }

    /// USDC on Ink (eip155:57073).
    #[must_use]
    pub fn ink() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(57073))
            .expect("built-in USDC deployment for Ink missing")
    }

    /// USDC on Ink Sepolia testnet (eip155:763373).
    #[must_use]
    pub fn ink_sepolia() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(763_373))
            .expect("built-in USDC deployment for Ink Sepolia missing")
    }

    /// USDC on Sei (eip155:1329).
    #[must_use]
    pub fn sei() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(1329))
            .expect("built-in USDC deployment for Sei missing")
    }

    /// USDC on Sei testnet (eip155:1328).
    #[must_use]
    pub fn sei_testnet() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(1328))
            .expect("built-in USDC deployment for Sei testnet missing")
    }

    /// USDC on HyperEVM (eip155:999).
    #[must_use]
    pub fn hyperevm() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(999))
            .expect("built-in USDC deployment for HyperEVM missing")
    }

    /// USDC on HyperEVM testnet (eip155:998).
    #[must_use]
    pub fn hyperevm_testnet() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(998))
            .expect("built-in USDC deployment for HyperEVM testnet missing")
    }

    /// USDC on Monad (eip155:143).
    #[must_use]
    pub fn monad() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(143))
            .expect("built-in USDC deployment for Monad missing")
    }

    /// USDC on Monad testnet (eip155:10143).
    #[must_use]
    pub fn monad_testnet() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(10143))
            .expect("built-in USDC deployment for Monad testnet missing")
    }

    /// USDC on Plume (eip155:98866).
    #[must_use]
    pub fn plume() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(98866))
            .expect("built-in USDC deployment for Plume missing")
    }

    /// USDC on Plume testnet (eip155:98867).
    #[must_use]
    pub fn plume_testnet() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(98867))
            .expect("built-in USDC deployment for Plume testnet missing")
    }

    /// USDC on Codex (eip155:81224).
    #[must_use]
    pub fn codex() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(81224))
            .expect("built-in USDC deployment for Codex missing")
    }

    /// USDC on Codex testnet (eip155:812242).
    #[must_use]
    pub fn codex_testnet() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(812_242))
            .expect("built-in USDC deployment for Codex testnet missing")
    }

    /// USDC on XDC Network (eip155:50).
    #[must_use]
    pub fn xdc() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(50))
            .expect("built-in USDC deployment for XDC missing")
    }

    /// USDC on XDC Apothem testnet (eip155:51).
    #[must_use]
    pub fn xdc_apothem() -> &'static Eip155TokenDeployment {
        usdc_evm_deployment(&Eip155ChainReference::new(51))
            .expect("built-in USDC deployment for XDC Apothem missing")
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
pub struct USDM;

#[allow(clippy::doc_markdown, clippy::missing_panics_doc)]
impl USDM {
    /// Looks up a USDM deployment by chain reference.
    #[must_use]
    pub fn on(chain: &Eip155ChainReference) -> Option<&'static Eip155TokenDeployment> {
        usdm_evm_deployment(chain)
    }

    /// Returns all known USDM deployments.
    #[must_use]
    pub fn all() -> &'static [Eip155TokenDeployment] {
        usdm_evm_deployments()
    }

    /// USDM (MegaUSD) on MegaETH (eip155:4326).
    #[must_use]
    pub fn megaeth() -> &'static Eip155TokenDeployment {
        usdm_evm_deployment(&Eip155ChainReference::new(4326))
            .expect("built-in USDM deployment for MegaETH missing")
    }
}
