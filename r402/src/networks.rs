//! Known blockchain networks and CAIP-2 chain ID management.
//!
//! This module provides a registry of well-known blockchain networks with their
//! CAIP-2 chain identifiers, primarily for x402 v1 protocol compatibility and
//! improved developer experience. For v2+, the protocol works with any CAIP-2
//! chain ID without requiring a predefined registry.
//!
//! The registry powers [`ChainId::from_network_name()`](crate::chain::ChainId::from_network_name)
//! and [`ChainId::as_network_name()`](crate::chain::ChainId::as_network_name) lookups.
//! Chain-specific crates (`r402-evm`, `r402-svm`) extend this with namespace-specific
//! traits and token deployment data via the [`USDC`] marker struct.
//!

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::chain::ChainId;

/// A known network definition with its chain ID and human-readable name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkInfo {
    /// Human-readable network name (e.g., "base-sepolia", "solana")
    pub name: &'static str,
    /// CAIP-2 namespace (e.g., "eip155", "solana")
    pub namespace: &'static str,
    /// Chain reference (e.g., "84532" for Base Sepolia, "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp" for Solana mainnet)
    pub reference: &'static str,
}

impl NetworkInfo {
    /// Create a `ChainId` from this network info
    #[must_use]
    pub fn chain_id(&self) -> ChainId {
        ChainId::new(self.namespace, self.reference)
    }
}

/// Registry of well-known blockchain networks, organized by ecosystem.
///
/// Populates [`NAME_TO_CHAIN_ID`] and [`CHAIN_ID_TO_NAME`] lookup tables.
///
/// Source: <https://developers.circle.com/stablecoins/usdc-contract-addresses>
pub static KNOWN_NETWORKS: &[NetworkInfo] = &[
    // Ethereum — https://etherscan.io/token/0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48
    NetworkInfo {
        name: "ethereum",
        namespace: "eip155",
        reference: "1",
    },
    // Ethereum Sepolia — https://sepolia.etherscan.io/address/0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238
    NetworkInfo {
        name: "ethereum-sepolia",
        namespace: "eip155",
        reference: "11155111",
    },
    // Base — https://basescan.org/token/0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913
    NetworkInfo {
        name: "base",
        namespace: "eip155",
        reference: "8453",
    },
    // Base Sepolia — https://base-sepolia.blockscout.com/address/0x036CbD53842c5426634e7929541eC2318f3dCF7e
    NetworkInfo {
        name: "base-sepolia",
        namespace: "eip155",
        reference: "84532",
    },
    // Arbitrum One — https://arbiscan.io/token/0xaf88d065e77c8cC2239327C5EDb3A432268e5831
    NetworkInfo {
        name: "arbitrum",
        namespace: "eip155",
        reference: "42161",
    },
    // Arbitrum Sepolia — https://sepolia.arbiscan.io/address/0x75faf114eafb1BDbe2F0316DF893fd58CE46AA4d
    NetworkInfo {
        name: "arbitrum-sepolia",
        namespace: "eip155",
        reference: "421614",
    },
    // OP Mainnet — https://optimistic.etherscan.io/token/0x0b2c639c533813f4aa9d7837caf62653d097ff85
    NetworkInfo {
        name: "optimism",
        namespace: "eip155",
        reference: "10",
    },
    // OP Sepolia — https://sepolia-optimism.etherscan.io/address/0x5fd84259d66Cd46123540766Be93DFE6D43130D7
    NetworkInfo {
        name: "optimism-sepolia",
        namespace: "eip155",
        reference: "11155420",
    },
    // Polygon PoS — https://polygonscan.com/token/0x3c499c542cef5e3811e1192ce70d8cc03d5c3359
    NetworkInfo {
        name: "polygon",
        namespace: "eip155",
        reference: "137",
    },
    // Polygon Amoy — https://amoy.polygonscan.com/address/0x41e94eb019c0762f9bfcf9fb1e58725bfb0e7582
    NetworkInfo {
        name: "polygon-amoy",
        namespace: "eip155",
        reference: "80002",
    },
    // Avalanche C-Chain — https://snowtrace.io/token/0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E
    NetworkInfo {
        name: "avalanche",
        namespace: "eip155",
        reference: "43114",
    },
    // Avalanche Fuji — https://testnet.snowtrace.io/token/0x5425890298aed601595a70ab815c96711a31bc65
    NetworkInfo {
        name: "avalanche-fuji",
        namespace: "eip155",
        reference: "43113",
    },
    // Celo — https://celoscan.io/token/0xcebA9300f2b948710d2653dD7B07f33A8B32118C
    NetworkInfo {
        name: "celo",
        namespace: "eip155",
        reference: "42220",
    },
    // Celo Sepolia — https://celo-sepolia.blockscout.com/token/0x01C5C0122039549AD1493B8220cABEdD739BC44E
    NetworkInfo {
        name: "celo-sepolia",
        namespace: "eip155",
        reference: "11142220",
    },
    // Sei — https://seitrace.com/address/0xe15fC38F6D8c56aF07bbCBe3BAf5708A2Bf42392?chain=pacific-1
    NetworkInfo {
        name: "sei",
        namespace: "eip155",
        reference: "1329",
    },
    // Sei Testnet — https://seitrace.com/address/0x4fCF1784B31630811181f670Aea7A7bEF803eaED?chain=atlantic-2
    NetworkInfo {
        name: "sei-testnet",
        namespace: "eip155",
        reference: "1328",
    },
    // Sonic — https://sonicscan.org/token/0x29219dd400f2bf60e5a23d13be72b486d4038894
    NetworkInfo {
        name: "sonic",
        namespace: "eip155",
        reference: "146",
    },
    // Sonic Blaze Testnet — https://blaze.soniclabs.com/address/0xA4879Fed32Ecbef99399e5cbC247E533421C4eC6
    NetworkInfo {
        name: "sonic-blaze",
        namespace: "eip155",
        reference: "57054",
    },
    // Unichain — https://uniscan.xyz/token/0x078d782b760474a361dda0af3839290b0ef57ad6
    NetworkInfo {
        name: "unichain",
        namespace: "eip155",
        reference: "130",
    },
    // Unichain Sepolia — https://unichain-sepolia.blockscout.com/token/0x31d0220469e10c4E71834a79b1f276d740d3768F
    NetworkInfo {
        name: "unichain-sepolia",
        namespace: "eip155",
        reference: "1301",
    },
    // World Chain — https://worldscan.org/address/0x79A02482A880bCe3F13E09da970dC34dB4cD24D1
    NetworkInfo {
        name: "world-chain",
        namespace: "eip155",
        reference: "480",
    },
    // World Chain Sepolia — https://sepolia.worldscan.org/address/0x66145f38cBAC35Ca6F1Dfb4914dF98F1614aeA88
    NetworkInfo {
        name: "world-chain-sepolia",
        namespace: "eip155",
        reference: "4801",
    },
    // ZKsync Era — https://explorer.zksync.io/address/0x1d17CBcF0D6D143135aE902365D2E5e2A16538D4
    NetworkInfo {
        name: "zksync",
        namespace: "eip155",
        reference: "324",
    },
    // ZKsync Era Sepolia — https://sepolia.explorer.zksync.io/address/0xAe045DE5638162fa134807Cb558E15A3F5A7F853
    NetworkInfo {
        name: "zksync-sepolia",
        namespace: "eip155",
        reference: "300",
    },
    // Linea — https://lineascan.build/token/0x176211869ca2b568f2a7d4ee941e073a821ee1ff
    NetworkInfo {
        name: "linea",
        namespace: "eip155",
        reference: "59144",
    },
    // Linea Sepolia — https://sepolia.lineascan.build/address/0xFEce4462D57bD51A6A552365A011b95f0E16d9B7
    NetworkInfo {
        name: "linea-sepolia",
        namespace: "eip155",
        reference: "59141",
    },
    // Ink — https://explorer.inkonchain.com/address/0x2D270e6886d130D724215A266106e6832161EAEd
    NetworkInfo {
        name: "ink",
        namespace: "eip155",
        reference: "57073",
    },
    // Ink Sepolia — https://explorer-sepolia.inkonchain.com/address/0xFabab97dCE620294D2B0b0e46C68964e326300Ac
    NetworkInfo {
        name: "ink-sepolia",
        namespace: "eip155",
        reference: "763373",
    },
    // HyperEVM — https://hyperscan.com/token/0xb88339CB7199b77E23DB6E890353E22632Ba630f
    NetworkInfo {
        name: "hyperevm",
        namespace: "eip155",
        reference: "999",
    },
    // HyperEVM Testnet — https://testnet.purrsec.com/address/0x2B3370eE501B4a559b57D449569354196457D8Ab
    NetworkInfo {
        name: "hyperevm-testnet",
        namespace: "eip155",
        reference: "998",
    },
    // Monad — https://monadvision.com/token/0x754704Bc059F8C67012fEd69BC8A327a5aafb603
    NetworkInfo {
        name: "monad",
        namespace: "eip155",
        reference: "143",
    },
    // Monad Testnet — https://testnet.monadvision.com/token/0x534b2f3A21130d7a60830c2Df862319e593943A3
    NetworkInfo {
        name: "monad-testnet",
        namespace: "eip155",
        reference: "10143",
    },
    // Plume — https://explorer.plume.org/address/0x222365EF19F7947e5484218551B56bb3965Aa7aF
    NetworkInfo {
        name: "plume",
        namespace: "eip155",
        reference: "98866",
    },
    // Plume Testnet — https://testnet-explorer.plume.org/address/0xcB5f30e335672893c7eb944B374c196392C19D18
    NetworkInfo {
        name: "plume-testnet",
        namespace: "eip155",
        reference: "98867",
    },
    // Codex — https://explorer.codex.xyz/address/0xd996633a415985DBd7D6D12f4A4343E31f5037cf
    NetworkInfo {
        name: "codex",
        namespace: "eip155",
        reference: "81224",
    },
    // Codex Testnet — https://explorer.codex-stg.xyz/address/0x6d7f141b6819C2c9CC2f818e6ad549E7Ca090F8f
    NetworkInfo {
        name: "codex-testnet",
        namespace: "eip155",
        reference: "812242",
    },
    // XDC — https://xdcscan.com/address/0xfA2958CB79b0491CC627c1557F441eF849Ca8eb1
    NetworkInfo {
        name: "xdc",
        namespace: "eip155",
        reference: "50",
    },
    // XDC Apothem — https://testnet.xdcscan.com/address/0xb5AB69F7bBada22B28e79C8FFAECe55eF1c771D4
    NetworkInfo {
        name: "xdc-apothem",
        namespace: "eip155",
        reference: "51",
    },
    // XRPL EVM — community deployment, not on Circle official page
    NetworkInfo {
        name: "xrpl-evm",
        namespace: "eip155",
        reference: "1440000",
    },
    // Peaq — community deployment, not on Circle official page
    NetworkInfo {
        name: "peaq",
        namespace: "eip155",
        reference: "3338",
    },
    // IoTeX — community deployment, not on Circle official page
    NetworkInfo {
        name: "iotex",
        namespace: "eip155",
        reference: "4689",
    },
    // Solana — https://solscan.io/token/EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v
    NetworkInfo {
        name: "solana",
        namespace: "solana",
        reference: "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp",
    },
    // Solana Devnet — https://explorer.solana.com/address/4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU?cluster=devnet
    NetworkInfo {
        name: "solana-devnet",
        namespace: "solana",
        reference: "EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
    },
];

/// Maps network names to [`ChainId`] instances, lazily built from [`KNOWN_NETWORKS`].
pub static NAME_TO_CHAIN_ID: LazyLock<HashMap<&'static str, ChainId>> = LazyLock::new(|| {
    KNOWN_NETWORKS
        .iter()
        .map(|n| (n.name, n.chain_id()))
        .collect()
});

/// Maps [`ChainId`] instances to network names, lazily built from [`KNOWN_NETWORKS`].
pub static CHAIN_ID_TO_NAME: LazyLock<HashMap<ChainId, &'static str>> = LazyLock::new(|| {
    KNOWN_NETWORKS
        .iter()
        .map(|n| (n.chain_id(), n.name))
        .collect()
});

/// Retrieves a [`ChainId`] by its human-readable network name (case-sensitive).
pub fn chain_id_by_network_name(name: &str) -> Option<&ChainId> {
    NAME_TO_CHAIN_ID.get(name)
}

/// Retrieves a human-readable network name by its [`ChainId`].
pub fn network_name_by_chain_id(chain_id: &ChainId) -> Option<&'static str> {
    CHAIN_ID_TO_NAME.get(chain_id).copied()
}

/// Marker struct for USDC token deployment implementations.
///
/// This struct is used as a type parameter for chain-specific traits (e.g., `KnownNetworkEip155`,
/// `KnownNetworkSolana`) to provide per-network USDC token deployment information.
///
/// # Usage
///
/// Chain-specific crates implement traits for this marker struct to provide USDC token
/// deployments on different networks. For example:
///
/// - `r402-evm` implements `KnownNetworkEip155<Eip155TokenDeployment>` for `USDC`
/// - `r402-svm` implements `KnownNetworkSolana<SolanaTokenDeployment>` for `USDC`
#[derive(Debug, Clone, Copy)]
#[allow(clippy::upper_case_acronyms)]
pub struct USDC;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_id_from_network_name() {
        let base = chain_id_by_network_name("base").unwrap();
        assert_eq!(base.namespace, "eip155");
        assert_eq!(base.reference, "8453");

        let base_sepolia = chain_id_by_network_name("base-sepolia").unwrap();
        assert_eq!(base_sepolia.namespace, "eip155");
        assert_eq!(base_sepolia.reference, "84532");

        let polygon = chain_id_by_network_name("polygon").unwrap();
        assert_eq!(polygon.namespace, "eip155");
        assert_eq!(polygon.reference, "137");

        let celo = chain_id_by_network_name("celo").unwrap();
        assert_eq!(celo.namespace, "eip155");
        assert_eq!(celo.reference, "42220");

        let solana = chain_id_by_network_name("solana").unwrap();
        assert_eq!(solana.namespace, "solana");
        assert_eq!(solana.reference, "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp");

        assert!(chain_id_by_network_name("unknown").is_none());
    }

    #[test]
    fn test_network_name_by_chain_id() {
        let chain_id = ChainId::new("eip155", "8453");
        let network_name = network_name_by_chain_id(&chain_id).unwrap();
        assert_eq!(network_name, "base");

        let celo_chain_id = ChainId::new("eip155", "42220");
        let network_name = network_name_by_chain_id(&celo_chain_id).unwrap();
        assert_eq!(network_name, "celo");

        let celo_sepolia_chain_id = ChainId::new("eip155", "11142220");
        let network_name = network_name_by_chain_id(&celo_sepolia_chain_id).unwrap();
        assert_eq!(network_name, "celo-sepolia");

        let solana_chain_id = ChainId::new("solana", "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp");
        let network_name = network_name_by_chain_id(&solana_chain_id).unwrap();
        assert_eq!(network_name, "solana");

        let unknown_chain_id = ChainId::new("eip155", "999999");
        assert!(network_name_by_chain_id(&unknown_chain_id).is_none());
    }

    #[test]
    fn test_chain_id_as_network_name() {
        let chain_id = ChainId::new("eip155", "8453");
        assert_eq!(chain_id.as_network_name(), Some("base"));

        let celo_chain_id = ChainId::new("eip155", "42220");
        assert_eq!(celo_chain_id.as_network_name(), Some("celo"));

        let solana_chain_id = ChainId::new("solana", "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp");
        assert_eq!(solana_chain_id.as_network_name(), Some("solana"));

        let unknown_chain_id = ChainId::new("eip155", "999999");
        assert!(unknown_chain_id.as_network_name().is_none());
    }
}
