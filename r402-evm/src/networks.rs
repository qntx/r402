use std::sync::LazyLock;

use r402::chain::ChainId;
use r402::networks::{NetworkInfo, NetworkRegistry, USDC, USDM};

use crate::chain::{Eip155ChainReference, Eip155TokenDeployment, TokenDeploymentEip712};

/// Well-known EVM (EIP-155) networks with their V1 names and CAIP-2 identifiers.
///
/// This is the **single source of truth** for EVM network name ↔ chain ID mappings.
/// Use [`evm_network_registry()`] for convenient lookups, or pass this slice to
/// [`NetworkRegistry::from_networks()`] to build a combined cross-chain registry.
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

static EVM_REGISTRY: LazyLock<NetworkRegistry> =
    LazyLock::new(|| NetworkRegistry::from_networks(EVM_NETWORKS));

/// Returns a lazily-initialized [`NetworkRegistry`] containing all known EVM networks.
///
/// This is a convenience accessor for V1 code paths within the `r402-evm` crate.
/// For cross-chain registries, build your own [`NetworkRegistry`] from [`EVM_NETWORKS`].
#[must_use]
pub fn evm_network_registry() -> &'static NetworkRegistry {
    &EVM_REGISTRY
}

/// Trait providing convenient methods to get instances for well-known EVM networks (eip155 namespace).
///
/// This trait can be implemented for any type to provide static methods that create
/// instances for well-known EVM blockchain networks. Each method returns `Self`, allowing
/// the trait to be used with different types that need per-network configuration.
///
/// # Use Cases
///
/// - **`ChainId`**: Get CAIP-2 chain identifiers for EVM networks
/// - **Token Deployments**: Get per-chain token addresses (e.g., USDC on different EVM chains)
/// - **Network Configuration**: Get network-specific configuration objects for EVM chains
/// - **Any Per-Network Data**: Any type that needs EVM network-specific instances
/// # Default implementations
///
/// Every method returns `None` by default so that implementors only need
/// to override networks where they actually have a deployment.
/// This avoids massive boilerplate for token types that are only deployed
/// on a subset of chains (e.g., USDM on `MegaETH` only).
///
/// For exhaustive types like [`ChainId`](r402::chain::ChainId), all methods
/// should be overridden.
#[allow(clippy::must_use_candidate)]
pub trait KnownNetworkEip155<A> {
    /// Returns the instance for Ethereum mainnet (eip155:1)
    fn ethereum() -> Option<A> {
        None
    }
    /// Returns the instance for Ethereum Sepolia testnet (eip155:11155111)
    fn ethereum_sepolia() -> Option<A> {
        None
    }

    /// Returns the instance for Base mainnet (eip155:8453)
    fn base() -> Option<A> {
        None
    }
    /// Returns the instance for Base Sepolia testnet (eip155:84532)
    fn base_sepolia() -> Option<A> {
        None
    }

    /// Returns the instance for Arbitrum One mainnet (eip155:42161)
    fn arbitrum() -> Option<A> {
        None
    }
    /// Returns the instance for Arbitrum Sepolia testnet (eip155:421614)
    fn arbitrum_sepolia() -> Option<A> {
        None
    }

    /// Returns the instance for OP Mainnet (eip155:10)
    fn optimism() -> Option<A> {
        None
    }
    /// Returns the instance for OP Sepolia testnet (eip155:11155420)
    fn optimism_sepolia() -> Option<A> {
        None
    }

    /// Returns the instance for Polygon `PoS` mainnet (eip155:137)
    fn polygon() -> Option<A> {
        None
    }
    /// Returns the instance for Polygon Amoy testnet (eip155:80002)
    fn polygon_amoy() -> Option<A> {
        None
    }

    /// Returns the instance for Avalanche C-Chain mainnet (eip155:43114)
    fn avalanche() -> Option<A> {
        None
    }
    /// Returns the instance for Avalanche Fuji testnet (eip155:43113)
    fn avalanche_fuji() -> Option<A> {
        None
    }

    /// Returns the instance for Celo mainnet (eip155:42220)
    fn celo() -> Option<A> {
        None
    }
    /// Returns the instance for Celo Sepolia testnet (eip155:11142220)
    fn celo_sepolia() -> Option<A> {
        None
    }

    /// Returns the instance for Sei mainnet (eip155:1329)
    fn sei() -> Option<A> {
        None
    }
    /// Returns the instance for Sei testnet (eip155:1328)
    fn sei_testnet() -> Option<A> {
        None
    }

    /// Returns the instance for Sonic mainnet (eip155:146)
    fn sonic() -> Option<A> {
        None
    }
    /// Returns the instance for Sonic Blaze testnet (eip155:57054)
    fn sonic_blaze() -> Option<A> {
        None
    }

    /// Returns the instance for Unichain mainnet (eip155:130)
    fn unichain() -> Option<A> {
        None
    }
    /// Returns the instance for Unichain Sepolia testnet (eip155:1301)
    fn unichain_sepolia() -> Option<A> {
        None
    }

    /// Returns the instance for World Chain mainnet (eip155:480)
    fn world_chain() -> Option<A> {
        None
    }
    /// Returns the instance for World Chain Sepolia testnet (eip155:4801)
    fn world_chain_sepolia() -> Option<A> {
        None
    }

    /// Returns the instance for `ZKsync` Era mainnet (eip155:324)
    fn zksync() -> Option<A> {
        None
    }
    /// Returns the instance for `ZKsync` Era Sepolia testnet (eip155:300)
    fn zksync_sepolia() -> Option<A> {
        None
    }

    /// Returns the instance for Linea mainnet (eip155:59144)
    fn linea() -> Option<A> {
        None
    }
    /// Returns the instance for Linea Sepolia testnet (eip155:59141)
    fn linea_sepolia() -> Option<A> {
        None
    }

    /// Returns the instance for Ink mainnet (eip155:57073)
    fn ink() -> Option<A> {
        None
    }
    /// Returns the instance for Ink Sepolia testnet (eip155:763373)
    fn ink_sepolia() -> Option<A> {
        None
    }

    /// Returns the instance for `HyperEVM` mainnet (eip155:999)
    fn hyperevm() -> Option<A> {
        None
    }
    /// Returns the instance for `HyperEVM` testnet (eip155:998)
    fn hyperevm_testnet() -> Option<A> {
        None
    }

    /// Returns the instance for Monad mainnet (eip155:143)
    fn monad() -> Option<A> {
        None
    }
    /// Returns the instance for Monad testnet (eip155:10143)
    fn monad_testnet() -> Option<A> {
        None
    }

    /// Returns the instance for Plume mainnet (eip155:98866)
    fn plume() -> Option<A> {
        None
    }
    /// Returns the instance for Plume testnet (eip155:98867)
    fn plume_testnet() -> Option<A> {
        None
    }

    /// Returns the instance for Codex mainnet (eip155:81224)
    fn codex() -> Option<A> {
        None
    }
    /// Returns the instance for Codex testnet (eip155:812242)
    fn codex_testnet() -> Option<A> {
        None
    }

    /// Returns the instance for XDC Network mainnet (eip155:50)
    fn xdc() -> Option<A> {
        None
    }
    /// Returns the instance for XDC Apothem testnet (eip155:51)
    fn xdc_apothem() -> Option<A> {
        None
    }

    /// Returns the instance for XRPL EVM sidechain (eip155:1440000)
    fn xrpl_evm() -> Option<A> {
        None
    }

    /// Returns the instance for Peaq mainnet (eip155:3338)
    fn peaq() -> Option<A> {
        None
    }

    /// Returns the instance for `IoTeX` mainnet (eip155:4689)
    fn iotex() -> Option<A> {
        None
    }

    /// Returns the instance for `MegaETH` mainnet (eip155:4326)
    fn megaeth() -> Option<A> {
        None
    }
}

/// Implementation of `KnownNetworkEip155` for `ChainId`.
///
/// Provides convenient static methods to create `ChainId` instances for well-known
/// EVM blockchain networks. Each method returns a properly configured `ChainId` with the
/// "eip155" namespace and the correct chain reference.
///
/// This is one example of implementing the `KnownNetworkEip155` trait. Other types
/// (such as token address types) can also implement this trait to provide
/// per-network instances with better developer experience.
impl KnownNetworkEip155<Self> for ChainId {
    fn ethereum() -> Option<Self> {
        Some(Self::new("eip155", "1"))
    }
    fn ethereum_sepolia() -> Option<Self> {
        Some(Self::new("eip155", "11155111"))
    }
    fn base() -> Option<Self> {
        Some(Self::new("eip155", "8453"))
    }
    fn base_sepolia() -> Option<Self> {
        Some(Self::new("eip155", "84532"))
    }
    fn arbitrum() -> Option<Self> {
        Some(Self::new("eip155", "42161"))
    }
    fn arbitrum_sepolia() -> Option<Self> {
        Some(Self::new("eip155", "421614"))
    }
    fn optimism() -> Option<Self> {
        Some(Self::new("eip155", "10"))
    }
    fn optimism_sepolia() -> Option<Self> {
        Some(Self::new("eip155", "11155420"))
    }
    fn polygon() -> Option<Self> {
        Some(Self::new("eip155", "137"))
    }
    fn polygon_amoy() -> Option<Self> {
        Some(Self::new("eip155", "80002"))
    }
    fn avalanche() -> Option<Self> {
        Some(Self::new("eip155", "43114"))
    }
    fn avalanche_fuji() -> Option<Self> {
        Some(Self::new("eip155", "43113"))
    }
    fn celo() -> Option<Self> {
        Some(Self::new("eip155", "42220"))
    }
    fn celo_sepolia() -> Option<Self> {
        Some(Self::new("eip155", "11142220"))
    }
    fn sei() -> Option<Self> {
        Some(Self::new("eip155", "1329"))
    }
    fn sei_testnet() -> Option<Self> {
        Some(Self::new("eip155", "1328"))
    }
    fn sonic() -> Option<Self> {
        Some(Self::new("eip155", "146"))
    }
    fn sonic_blaze() -> Option<Self> {
        Some(Self::new("eip155", "57054"))
    }
    fn unichain() -> Option<Self> {
        Some(Self::new("eip155", "130"))
    }
    fn unichain_sepolia() -> Option<Self> {
        Some(Self::new("eip155", "1301"))
    }
    fn world_chain() -> Option<Self> {
        Some(Self::new("eip155", "480"))
    }
    fn world_chain_sepolia() -> Option<Self> {
        Some(Self::new("eip155", "4801"))
    }
    fn zksync() -> Option<Self> {
        Some(Self::new("eip155", "324"))
    }
    fn zksync_sepolia() -> Option<Self> {
        Some(Self::new("eip155", "300"))
    }
    fn linea() -> Option<Self> {
        Some(Self::new("eip155", "59144"))
    }
    fn linea_sepolia() -> Option<Self> {
        Some(Self::new("eip155", "59141"))
    }
    fn ink() -> Option<Self> {
        Some(Self::new("eip155", "57073"))
    }
    fn ink_sepolia() -> Option<Self> {
        Some(Self::new("eip155", "763373"))
    }
    fn hyperevm() -> Option<Self> {
        Some(Self::new("eip155", "999"))
    }
    fn hyperevm_testnet() -> Option<Self> {
        Some(Self::new("eip155", "998"))
    }
    fn monad() -> Option<Self> {
        Some(Self::new("eip155", "143"))
    }
    fn monad_testnet() -> Option<Self> {
        Some(Self::new("eip155", "10143"))
    }
    fn plume() -> Option<Self> {
        Some(Self::new("eip155", "98866"))
    }
    fn plume_testnet() -> Option<Self> {
        Some(Self::new("eip155", "98867"))
    }
    fn codex() -> Option<Self> {
        Some(Self::new("eip155", "81224"))
    }
    fn codex_testnet() -> Option<Self> {
        Some(Self::new("eip155", "812242"))
    }
    fn xdc() -> Option<Self> {
        Some(Self::new("eip155", "50"))
    }
    fn xdc_apothem() -> Option<Self> {
        Some(Self::new("eip155", "51"))
    }
    fn xrpl_evm() -> Option<Self> {
        Some(Self::new("eip155", "1440000"))
    }
    fn peaq() -> Option<Self> {
        Some(Self::new("eip155", "3338"))
    }
    fn iotex() -> Option<Self> {
        Some(Self::new("eip155", "4689"))
    }
    fn megaeth() -> Option<Self> {
        Some(Self::new("eip155", "4326"))
    }
}

impl KnownNetworkEip155<Eip155TokenDeployment> for USDM {
    // MegaETH — MegaUSD (USDM), the chain's endorsed default stablecoin
    // Matches Go SDK: eip155:4326, name "MegaUSD", version "1", decimals 18
    fn megaeth() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(4326),
            address: alloy_primitives::address!("0xFAfDdbb3FC7688494971a79cc65DCa3EF82079E7"),
            decimals: 18,
            eip712: Some(TokenDeploymentEip712 {
                name: "MegaUSD".into(),
                version: "1".into(),
            }),
        })
    }
}

impl KnownNetworkEip155<Eip155TokenDeployment> for USDC {
    // Ethereum mainnet — native Circle USDC (FiatTokenV2.1)
    // Verify: https://etherscan.io/token/0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48
    fn ethereum() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(1),
            address: alloy_primitives::address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USD Coin".into(),
                version: "2".into(),
            }),
        })
    }

    // Ethereum Sepolia — native Circle USDC testnet
    // Verify: https://sepolia.etherscan.io/address/0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238
    fn ethereum_sepolia() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(11_155_111),
            address: alloy_primitives::address!("0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Base mainnet — native Circle USDC
    // Verify: https://basescan.org/token/0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913
    fn base() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(8453),
            address: alloy_primitives::address!("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USD Coin".into(),
                version: "2".into(),
            }),
        })
    }

    // Base Sepolia — native Circle USDC testnet
    // Verify: https://base-sepolia.blockscout.com/address/0x036CbD53842c5426634e7929541eC2318f3dCF7e
    fn base_sepolia() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(84532),
            address: alloy_primitives::address!("0x036CbD53842c5426634e7929541eC2318f3dCF7e"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Arbitrum One — native Circle USDC
    // Verify: https://arbiscan.io/token/0xaf88d065e77c8cC2239327C5EDb3A432268e5831
    fn arbitrum() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(42161),
            address: alloy_primitives::address!("0xaf88d065e77c8cC2239327C5EDb3A432268e5831"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USD Coin".into(),
                version: "2".into(),
            }),
        })
    }

    // Arbitrum Sepolia — native Circle USDC testnet
    // Verify: https://sepolia.arbiscan.io/address/0x75faf114eafb1BDbe2F0316DF893fd58CE46AA4d
    fn arbitrum_sepolia() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(421_614),
            address: alloy_primitives::address!("0x75faf114eafb1BDbe2F0316DF893fd58CE46AA4d"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // OP Mainnet — native Circle USDC
    // Verify: https://optimistic.etherscan.io/token/0x0b2c639c533813f4aa9d7837caf62653d097ff85
    fn optimism() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(10),
            address: alloy_primitives::address!("0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USD Coin".into(),
                version: "2".into(),
            }),
        })
    }

    // OP Sepolia — native Circle USDC testnet
    // Verify: https://sepolia-optimism.etherscan.io/address/0x5fd84259d66Cd46123540766Be93DFE6D43130D7
    fn optimism_sepolia() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(11_155_420),
            address: alloy_primitives::address!("0x5fd84259d66Cd46123540766Be93DFE6D43130D7"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Polygon PoS — native Circle USDC (not the old bridged USDC.e at 0x2791...)
    // Verify: https://polygonscan.com/token/0x3c499c542cef5e3811e1192ce70d8cc03d5c3359
    fn polygon() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(137),
            address: alloy_primitives::address!("0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Polygon Amoy — native Circle USDC testnet
    // Verify: https://amoy.polygonscan.com/address/0x41e94eb019c0762f9bfcf9fb1e58725bfb0e7582
    fn polygon_amoy() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(80002),
            address: alloy_primitives::address!("0x41E94Eb019C0762f9Bfcf9Fb1E58725BfB0e7582"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Avalanche C-Chain — native Circle USDC
    // Verify: https://snowtrace.io/token/0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E
    fn avalanche() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(43114),
            address: alloy_primitives::address!("0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USD Coin".into(),
                version: "2".into(),
            }),
        })
    }

    // Avalanche Fuji — native Circle USDC testnet
    // Verify: https://testnet.snowtrace.io/token/0x5425890298aed601595a70ab815c96711a31bc65
    fn avalanche_fuji() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(43113),
            address: alloy_primitives::address!("0x5425890298aed601595a70AB815c96711a31Bc65"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USD Coin".into(),
                version: "2".into(),
            }),
        })
    }

    // Celo — native Circle USDC
    // Verify: https://celoscan.io/token/0xcebA9300f2b948710d2653dD7B07f33A8B32118C
    fn celo() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(42220),
            address: alloy_primitives::address!("0xcebA9300f2b948710d2653dD7B07f33A8B32118C"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Celo Sepolia — native Circle USDC testnet
    // Verify: https://celo-sepolia.blockscout.com/token/0x01C5C0122039549AD1493B8220cABEdD739BC44E
    fn celo_sepolia() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(11_142_220),
            address: alloy_primitives::address!("0x01C5C0122039549AD1493B8220cABEdD739BC44E"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Sei — native Circle USDC
    // Verify: https://seitrace.com/address/0xe15fC38F6D8c56aF07bbCBe3BAf5708A2Bf42392?chain=pacific-1
    fn sei() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(1329),
            address: alloy_primitives::address!("0xe15fC38F6D8c56aF07bbCBe3BAf5708A2Bf42392"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Sei Testnet — native Circle USDC testnet
    // Verify: https://seitrace.com/address/0x4fCF1784B31630811181f670Aea7A7bEF803eaED?chain=atlantic-2
    fn sei_testnet() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(1328),
            address: alloy_primitives::address!("0x4fCF1784B31630811181f670Aea7A7bEF803eaED"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Sonic — native Circle USDC
    // Verify: https://sonicscan.org/token/0x29219dd400f2bf60e5a23d13be72b486d4038894
    fn sonic() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(146),
            address: alloy_primitives::address!("0x29219dd400f2Bf60E5a23d13Be72B486D4038894"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Sonic Blaze Testnet — native Circle USDC testnet
    // Verify: https://blaze.soniclabs.com/address/0xA4879Fed32Ecbef99399e5cbC247E533421C4eC6
    fn sonic_blaze() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(57054),
            address: alloy_primitives::address!("0xA4879Fed32Ecbef99399e5cbC247E533421C4eC6"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Unichain — native Circle USDC
    // Verify: https://uniscan.xyz/token/0x078d782b760474a361dda0af3839290b0ef57ad6
    fn unichain() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(130),
            address: alloy_primitives::address!("0x078D782b760474a361dDA0AF3839290b0EF57AD6"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Unichain Sepolia — native Circle USDC testnet
    // Verify: https://unichain-sepolia.blockscout.com/token/0x31d0220469e10c4E71834a79b1f276d740d3768F
    fn unichain_sepolia() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(1301),
            address: alloy_primitives::address!("0x31d0220469e10c4E71834a79b1f276d740d3768F"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // World Chain — native Circle USDC
    // Verify: https://worldscan.org/address/0x79A02482A880bCe3F13E09da970dC34dB4cD24D1
    fn world_chain() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(480),
            address: alloy_primitives::address!("0x79A02482A880bCe3F13E09da970dC34dB4cD24D1"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // World Chain Sepolia — native Circle USDC testnet
    // Verify: https://sepolia.worldscan.org/address/0x66145f38cBAC35Ca6F1Dfb4914dF98F1614aeA88
    fn world_chain_sepolia() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(4801),
            address: alloy_primitives::address!("0x66145f38cBAC35Ca6F1Dfb4914dF98F1614aeA88"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // ZKsync Era — native Circle USDC
    // Verify: https://explorer.zksync.io/address/0x1d17CBcF0D6D143135aE902365D2E5e2A16538D4
    fn zksync() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(324),
            address: alloy_primitives::address!("0x1d17CBcF0D6D143135aE902365D2E5e2A16538D4"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // ZKsync Era Sepolia — native Circle USDC testnet
    // Verify: https://sepolia.explorer.zksync.io/address/0xAe045DE5638162fa134807Cb558E15A3F5A7F853
    fn zksync_sepolia() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(300),
            address: alloy_primitives::address!("0xAe045DE5638162fa134807Cb558E15A3F5A7F853"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Linea — Circle USDC (upgraded from bridged to native via CCTP)
    // Verify: https://lineascan.build/token/0x176211869ca2b568f2a7d4ee941e073a821ee1ff
    fn linea() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(59144),
            address: alloy_primitives::address!("0x176211869cA2b568f2A7D4EE941E073a821EE1ff"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Linea Sepolia — Circle USDC testnet
    // Verify: https://sepolia.lineascan.build/address/0xFEce4462D57bD51A6A552365A011b95f0E16d9B7
    fn linea_sepolia() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(59141),
            address: alloy_primitives::address!("0xFEce4462D57bD51A6A552365A011b95f0E16d9B7"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Ink (by Kraken) — native Circle USDC
    // Verify: https://explorer.inkonchain.com/address/0x2D270e6886d130D724215A266106e6832161EAEd
    fn ink() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(57073),
            address: alloy_primitives::address!("0x2D270e6886d130D724215A266106e6832161EAEd"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Ink Sepolia — native Circle USDC testnet
    // Verify: https://explorer-sepolia.inkonchain.com/address/0xFabab97dCE620294D2B0b0e46C68964e326300Ac
    fn ink_sepolia() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(763_373),
            address: alloy_primitives::address!("0xFabab97dCE620294D2B0b0e46C68964e326300Ac"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // HyperEVM (Hyperliquid) — native Circle USDC
    // Verify: https://hyperscan.com/token/0xb88339CB7199b77E23DB6E890353E22632Ba630f
    fn hyperevm() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(999),
            address: alloy_primitives::address!("0xb88339CB7199b77E23DB6E890353E22632Ba630f"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // HyperEVM Testnet — native Circle USDC testnet
    // Verify: https://testnet.purrsec.com/address/0x2B3370eE501B4a559b57D449569354196457D8Ab
    fn hyperevm_testnet() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(998),
            address: alloy_primitives::address!("0x2B3370eE501B4a559b57D449569354196457D8Ab"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Monad — native Circle USDC
    // Verify: https://monadvision.com/token/0x754704Bc059F8C67012fEd69BC8A327a5aafb603
    fn monad() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(143),
            address: alloy_primitives::address!("0x754704Bc059F8C67012fEd69BC8A327a5aafb603"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Monad Testnet — native Circle USDC testnet
    // Verify: https://testnet.monadvision.com/token/0x534b2f3A21130d7a60830c2Df862319e593943A3
    fn monad_testnet() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(10143),
            address: alloy_primitives::address!("0x534b2f3A21130d7a60830c2Df862319e593943A3"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Plume — native Circle USDC
    // Verify: https://explorer.plume.org/address/0x222365EF19F7947e5484218551B56bb3965Aa7aF
    fn plume() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(98866),
            address: alloy_primitives::address!("0x222365EF19F7947e5484218551B56bb3965Aa7aF"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Plume Testnet — native Circle USDC testnet
    // Verify: https://testnet-explorer.plume.org/address/0xcB5f30e335672893c7eb944B374c196392C19D18
    fn plume_testnet() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(98867),
            address: alloy_primitives::address!("0xcB5f30e335672893c7eb944B374c196392C19D18"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Codex — native Circle USDC
    // Verify: https://explorer.codex.xyz/address/0xd996633a415985DBd7D6D12f4A4343E31f5037cf
    fn codex() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(81224),
            address: alloy_primitives::address!("0xd996633a415985DBd7D6D12f4A4343E31f5037cf"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // Codex Testnet — native Circle USDC testnet
    // Verify: https://explorer.codex-stg.xyz/address/0x6d7f141b6819C2c9CC2f818e6ad549E7Ca090F8f
    fn codex_testnet() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(812_242),
            address: alloy_primitives::address!("0x6d7f141b6819C2c9CC2f818e6ad549E7Ca090F8f"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // XDC Network — native Circle USDC
    // Verify: https://xdcscan.com/address/0xfA2958CB79b0491CC627c1557F441eF849Ca8eb1
    fn xdc() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(50),
            address: alloy_primitives::address!("0xfA2958CB79b0491CC627c1557F441eF849Ca8eb1"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // XDC Apothem Testnet — native Circle USDC testnet
    // Verify: https://testnet.xdcscan.com/address/0xb5AB69F7bBada22B28e79C8FFAECe55eF1c771D4
    fn xdc_apothem() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(51),
            address: alloy_primitives::address!("0xb5AB69F7bBada22B28e79C8FFAECe55eF1c771D4"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // XRPL EVM sidechain — community deployment, not on Circle official page
    // EIP-3009 support unverified (eip712: None)
    fn xrpl_evm() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(1_440_000),
            address: alloy_primitives::address!("0xDaF4556169c4F3f2231d8ab7BC8772Ddb7D4c84C"),
            decimals: 6,
            eip712: None,
        })
    }

    // Peaq — community deployment, not on Circle official page
    // EIP-3009 support unverified
    fn peaq() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(3338),
            address: alloy_primitives::address!("0xbbA60da06c2c5424f03f7434542280FCAd453d10"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "USDC".into(),
                version: "2".into(),
            }),
        })
    }

    // IoTeX — community deployment, not on Circle official page
    // EIP-3009 support unverified
    fn iotex() -> Option<Eip155TokenDeployment> {
        Some(Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(4689),
            address: alloy_primitives::address!("0xcdf79194c6c285077a58da47641d4dbe51f63542"),
            decimals: 6,
            eip712: Some(TokenDeploymentEip712 {
                name: "Bridged USDC".into(),
                version: "2".into(),
            }),
        })
    }
}
