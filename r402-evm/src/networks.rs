//! Known EVM network configurations and USDC token deployments.
//!
//! Corresponds to Python SDK's `NETWORK_CONFIGS` in `mechanisms/evm/constants.py`.

use alloy_primitives::{Address, address};

use crate::chain::{AssetInfo, ChainId, NetworkConfig};

/// Base Mainnet chain ID.
pub const BASE_MAINNET: ChainId = 8453;

/// Base Sepolia (testnet) chain ID.
pub const BASE_SEPOLIA: ChainId = 84532;

/// Polygon Mainnet chain ID.
pub const POLYGON_MAINNET: ChainId = 137;

/// Polygon Amoy (testnet) chain ID.
pub const POLYGON_AMOY: ChainId = 80002;

/// Avalanche C-Chain chain ID.
pub const AVALANCHE_MAINNET: ChainId = 43114;

/// Avalanche Fuji (testnet) chain ID.
pub const AVALANCHE_FUJI: ChainId = 43113;

/// Ethereum Mainnet chain ID.
pub const ETHEREUM_MAINNET: ChainId = 1;

/// Celo Mainnet chain ID.
pub const CELO_MAINNET: ChainId = 42220;

/// Celo Sepolia (testnet) chain ID.
pub const CELO_SEPOLIA: ChainId = 11_142_220;

/// `MegaETH` Mainnet (Frontier) chain ID.
pub const MEGAETH_MAINNET: ChainId = 4326;

/// `MegaETH` Testnet v2 chain ID.
pub const MEGAETH_TESTNET: ChainId = 6343;

/// Monad Mainnet chain ID.
pub const MONAD_MAINNET: ChainId = 143;

/// Monad Testnet chain ID.
pub const MONAD_TESTNET: ChainId = 10143;

/// USDC contract address on Base Mainnet.
pub const USDC_BASE: Address = address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913");

/// USDC contract address on Base Sepolia.
pub const USDC_BASE_SEPOLIA: Address = address!("036CbD53842c5426634e7929541eC2318f3dCF7e");

/// USDC contract address on Ethereum Mainnet.
pub const USDC_ETHEREUM: Address = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");

/// USDC contract address on Polygon Mainnet.
pub const USDC_POLYGON: Address = address!("3c499c542cEF5E3811e1192ce70d8cC03d5c3359");

/// USDC contract address on Polygon Amoy (Circle native).
pub const USDC_POLYGON_AMOY: Address = address!("41E94Eb019C0762f9Bfcf9Fb1E58725BfB0e7582");

/// USDC contract address on Avalanche C-Chain.
pub const USDC_AVALANCHE: Address = address!("B97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E");

/// USDC contract address on Avalanche Fuji.
pub const USDC_AVALANCHE_FUJI: Address = address!("5425890298aed601595a70AB815c96711a31Bc65");

/// USDC contract address on Celo.
pub const USDC_CELO: Address = address!("cebA9300f2b948710d2653dD7B07f33A8B32118C");

/// USDC contract address on Celo Sepolia (Circle native).
pub const USDC_CELO_SEPOLIA: Address = address!("01C5C0122039549AD1493B8220cABEdD739BC44E");

/// USDM contract address on `MegaETH` Mainnet (Frontier).
/// `MegaETH` uses USDM (`MegaUSD`) instead of Circle USDC.
pub const USDM_MEGAETH: Address = address!("FAfDdbb3FC7688494971a79cc65DCa3EF82079E7");

/// USDC contract address on Monad Mainnet (Circle native).
pub const USDC_MONAD: Address = address!("754704Bc059F8C67012fEd69BC8A327a5aafb603");

/// USDC contract address on Monad Testnet (Circle native).
pub const USDC_MONAD_TESTNET: Address = address!("534b2f3A21130d7a60830c2Df862319e593943A3");

/// Default EIP-712 domain name for USDC.
pub const DEFAULT_USDC_NAME: &str = "USD Coin";

/// Default EIP-712 domain version for USDC.
pub const DEFAULT_USDC_VERSION: &str = "2";

/// Default token decimals for USDC.
pub const DEFAULT_TOKEN_DECIMALS: u8 = 6;

/// Returns network configurations for all known EVM networks.
#[must_use]
pub fn known_networks() -> Vec<NetworkConfig> {
    vec![
        NetworkConfig {
            network: format!("eip155:{BASE_MAINNET}"),
            chain_id: BASE_MAINNET,
            assets: vec![usdc_asset(
                USDC_BASE,
                DEFAULT_USDC_NAME,
                DEFAULT_USDC_VERSION,
            )],
        },
        NetworkConfig {
            network: format!("eip155:{BASE_SEPOLIA}"),
            chain_id: BASE_SEPOLIA,
            assets: vec![usdc_asset(
                USDC_BASE_SEPOLIA,
                // Base Sepolia USDC contract returns "USDC" from name(), not "USD Coin".
                "USDC",
                DEFAULT_USDC_VERSION,
            )],
        },
        NetworkConfig {
            network: format!("eip155:{ETHEREUM_MAINNET}"),
            chain_id: ETHEREUM_MAINNET,
            assets: vec![usdc_asset(
                USDC_ETHEREUM,
                DEFAULT_USDC_NAME,
                DEFAULT_USDC_VERSION,
            )],
        },
        NetworkConfig {
            network: format!("eip155:{POLYGON_MAINNET}"),
            chain_id: POLYGON_MAINNET,
            assets: vec![usdc_asset(
                USDC_POLYGON,
                DEFAULT_USDC_NAME,
                DEFAULT_USDC_VERSION,
            )],
        },
        NetworkConfig {
            network: format!("eip155:{POLYGON_AMOY}"),
            chain_id: POLYGON_AMOY,
            assets: vec![usdc_asset(
                USDC_POLYGON_AMOY,
                // Polygon Amoy uses newer FiatTokenV2_2 with name() = "USDC".
                "USDC",
                DEFAULT_USDC_VERSION,
            )],
        },
        NetworkConfig {
            network: format!("eip155:{AVALANCHE_MAINNET}"),
            chain_id: AVALANCHE_MAINNET,
            assets: vec![usdc_asset(
                USDC_AVALANCHE,
                DEFAULT_USDC_NAME,
                DEFAULT_USDC_VERSION,
            )],
        },
        NetworkConfig {
            network: format!("eip155:{AVALANCHE_FUJI}"),
            chain_id: AVALANCHE_FUJI,
            assets: vec![usdc_asset(
                USDC_AVALANCHE_FUJI,
                DEFAULT_USDC_NAME,
                DEFAULT_USDC_VERSION,
            )],
        },
        NetworkConfig {
            network: format!("eip155:{CELO_MAINNET}"),
            chain_id: CELO_MAINNET,
            assets: vec![usdc_asset(
                USDC_CELO,
                // Celo USDC contract name() = "USDC" (confirmed on-chain).
                "USDC",
                DEFAULT_USDC_VERSION,
            )],
        },
        NetworkConfig {
            network: format!("eip155:{CELO_SEPOLIA}"),
            chain_id: CELO_SEPOLIA,
            assets: vec![usdc_asset(
                USDC_CELO_SEPOLIA,
                // Celo Sepolia uses Circle native USDC, name() = "USDC".
                "USDC",
                DEFAULT_USDC_VERSION,
            )],
        },
        NetworkConfig {
            network: format!("eip155:{MEGAETH_MAINNET}"),
            chain_id: MEGAETH_MAINNET,
            assets: vec![AssetInfo {
                address: USDM_MEGAETH,
                decimals: 18,
                name: "MegaUSD".to_owned(),
                version: "1".to_owned(),
            }],
        },
        NetworkConfig {
            network: format!("eip155:{MONAD_MAINNET}"),
            chain_id: MONAD_MAINNET,
            assets: vec![usdc_asset(
                USDC_MONAD,
                // Monad uses Circle native USDC (FiatTokenV2_2), name() = "USDC".
                "USDC",
                DEFAULT_USDC_VERSION,
            )],
        },
        NetworkConfig {
            network: format!("eip155:{MONAD_TESTNET}"),
            chain_id: MONAD_TESTNET,
            assets: vec![usdc_asset(
                USDC_MONAD_TESTNET,
                // Monad testnet uses Circle native USDC, name() = "USDC".
                "USDC",
                DEFAULT_USDC_VERSION,
            )],
        },
    ]
}

/// Returns all CAIP-2 network identifiers for known EVM networks.
#[must_use]
pub fn known_network_ids() -> Vec<String> {
    known_networks().into_iter().map(|n| n.network).collect()
}

fn usdc_asset(address: Address, name: &str, version: &str) -> AssetInfo {
    AssetInfo {
        address,
        decimals: DEFAULT_TOKEN_DECIMALS,
        name: name.to_owned(),
        version: version.to_owned(),
    }
}
