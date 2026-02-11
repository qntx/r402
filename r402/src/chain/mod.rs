//! Blockchain-specific types and providers for x402 payment processing.
//!
//! This module provides abstractions for interacting with different blockchain networks
//! in the x402 protocol.
//!
//! # Architecture
//!
//! The module is organized around the concept of chain providers and chain identifiers:
//!
//! - [`ChainId`] - A CAIP-2 compliant chain identifier (e.g., `eip155:8453` for Base)
//! - [`ChainIdPattern`] - Pattern matching for chain IDs (exact, wildcard, or set)
//! - [`ChainRegistry`] - Registry of configured chain providers

mod chain_id;

pub use chain_id::*;

use std::collections::HashMap;
use std::sync::Arc;

/// Common operations available on all chain providers.
///
/// This trait provides a unified interface for querying chain provider metadata
/// regardless of the underlying blockchain type.
pub trait ChainProviderOps {
    /// Returns the addresses of all configured signers for this chain.
    ///
    /// For EVM chains, these are Ethereum addresses (0x-prefixed hex).
    /// For Solana, these are base58-encoded public keys.
    fn signer_addresses(&self) -> Vec<String>;

    /// Returns the CAIP-2 chain identifier for this provider.
    fn chain_id(&self) -> ChainId;
}

impl<T: ChainProviderOps> ChainProviderOps for Arc<T> {
    fn signer_addresses(&self) -> Vec<String> {
        (**self).signer_addresses()
    }
    fn chain_id(&self) -> ChainId {
        (**self).chain_id()
    }
}

/// Registry of configured chain providers indexed by chain ID.
///
/// The registry is built from configuration and provides lookup methods
/// for finding providers by exact chain ID or by pattern matching.
///
/// # Type Parameters
///
/// - `P` - The chain provider type (e.g., `Eip155ChainProvider` or `SolanaChainProvider`)
#[derive(Debug)]
pub struct ChainRegistry<P>(HashMap<ChainId, P>);

impl<P> ChainRegistry<P> {
    /// Creates a new registry from the given provider map.
    ///
    /// # Parameters
    ///
    /// - `providers`: A map of chain IDs to providers.
    #[must_use]
    pub const fn new(providers: HashMap<ChainId, P>) -> Self {
        Self(providers)
    }
}

impl<P> ChainRegistry<P> {
    /// Looks up a provider by exact chain ID.
    ///
    /// Returns `None` if no provider is configured for the given chain.
    #[must_use]
    pub fn by_chain_id(&self, chain_id: &ChainId) -> Option<&P> {
        self.0.get(chain_id)
    }

    /// Looks up providers by chain ID pattern matching.
    ///
    /// Returns all providers whose chain IDs match the given pattern.
    /// The pattern can be:
    /// - Wildcard: Matches any chain within a namespace (e.g., `eip155:*`)
    /// - Exact: Matches a specific chain (e.g., `eip155:8453`)
    /// - Set: Matches any chain from a set of references (e.g., `eip155:{1,8453,137}`)
    #[must_use]
    pub fn by_chain_id_pattern(&self, pattern: &ChainIdPattern) -> Vec<&P> {
        self.0
            .iter()
            .filter_map(|(chain_id, provider)| pattern.matches(chain_id).then_some(provider))
            .collect()
    }
}

/// A token amount paired with its deployment information.
///
/// This type associates a numeric amount with the token deployment it refers to,
/// enabling type-safe handling of token amounts across different chains and tokens.
///
/// # Type Parameters
///
/// - `TAmount` - The numeric type for the amount (e.g., `U256` for EVM, `u64` for Solana)
/// - `TToken` - The token deployment type containing chain and address information
#[derive(Debug, Clone)]
pub struct DeployedTokenAmount<TAmount, TToken> {
    /// The token amount in the token's smallest unit (e.g., wei for ETH, lamports for SOL).
    pub amount: TAmount,
    /// The token deployment information including chain, address, and decimals.
    pub token: TToken,
}
