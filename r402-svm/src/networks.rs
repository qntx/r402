use r402::chain::ChainId;
use r402::networks::USDC;
use solana_pubkey::pubkey;

use crate::chain::{SolanaChainReference, SolanaTokenDeployment};

/// Trait providing convenient methods to get instances for well-known Solana networks.
///
/// This trait can be implemented for any type to provide static methods that create
/// instances for well-known Solana blockchain networks. Each method returns `Self`, allowing
/// the trait to be used with different types that need per-network configuration.
///
/// # Use Cases
///
/// - **`ChainId`**: Get CAIP-2 chain identifiers for Solana networks
/// - **Token Deployments**: Get per-chain token addresses (e.g., USDC on different Solana networks)
/// - **Network Configuration**: Get network-specific configuration objects for Solana chains
/// - **Any Per-Network Data**: Any type that needs Solana network-specific instances
pub trait KnownNetworkSolana<A> {
    /// Returns the instance for Solana mainnet (solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp)
    fn solana() -> A;
    /// Returns the instance for Solana devnet (solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1)
    fn solana_devnet() -> A;
}

/// Implementation of `KnownNetworkSolana` for `ChainId`.
///
/// Provides convenient static methods to create `ChainId` instances for well-known
/// Solana blockchain networks. Each method returns a properly configured `ChainId` with the
/// "solana" namespace and the correct chain reference.
///
/// This is one example of implementing the `KnownNetworkSolana` trait. Other types
/// (such as token address types) can also implement this trait to provide
/// per-network instances with better developer experience.
impl KnownNetworkSolana<Self> for ChainId {
    fn solana() -> Self {
        SolanaChainReference::solana().into()
    }

    fn solana_devnet() -> Self {
        SolanaChainReference::solana_devnet().into()
    }
}

impl KnownNetworkSolana<SolanaTokenDeployment> for USDC {
    // Solana mainnet — native Circle USDC (SPL Token)
    // Verify: https://solscan.io/token/EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v
    fn solana() -> SolanaTokenDeployment {
        let address = pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
        SolanaTokenDeployment::new(SolanaChainReference::solana(), address.into(), 6)
    }

    // Solana devnet — native Circle USDC testnet (SPL Token)
    // Verify: https://explorer.solana.com/address/4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU?cluster=devnet
    fn solana_devnet() -> SolanaTokenDeployment {
        let address = pubkey!("4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU");
        SolanaTokenDeployment::new(SolanaChainReference::solana_devnet(), address.into(), 6)
    }
}
