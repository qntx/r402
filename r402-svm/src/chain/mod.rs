//! Solana chain support for x402 payments.
//!
//! This module provides types and providers for interacting with the Solana blockchain
//! in the x402 protocol. It supports SPL token transfers for payment settlement.
//!
//! # Key Types
//!
//! - [`SolanaChainReference`] - A 32-character genesis hash identifying a Solana network
//! - [`SolanaChainProvider`] - Provider for interacting with Solana chains
//! - [`SolanaTokenDeployment`] - Token deployment information including mint address and decimals
//! - [`Address`] - A Solana public key (base58-encoded)
//!
//! # Solana Networks
//!
//! Solana networks are identified by the first 32 characters of their genesis block hash:
//! - Mainnet: `5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp`
//! - Devnet: `EtWTRABZaYq6iMfeYKouRu166VU2xqa1`
//!
//! # Example
//!
//! ```ignore
//! use r402_svm::chain::{SolanaChainReference, SolanaTokenDeployment};
//! use r402_svm::KnownNetworkSolana;
//! use r402::networks::USDC;
//!
//! // Get USDC deployment on Solana mainnet
//! let usdc = USDC::solana();
//! assert_eq!(usdc.decimals, 6);
//!
//! // Parse a human-readable amount
//! let amount = usdc.parse("10.50").unwrap();
//! // amount.amount is now 10_500_000 (10.50 * 10^6)
//! ```

/// Core Solana chain types (addresses, references, token deployments).
pub mod types;
pub use types::*;

/// Facilitator configuration for Solana chains.
#[cfg(feature = "facilitator")]
pub mod config;

/// Solana chain provider implementation.
#[cfg(feature = "facilitator")]
pub mod provider;
#[cfg(feature = "facilitator")]
pub use provider::*;

/// RPC client abstraction for Solana.
#[cfg(feature = "client")]
pub mod rpc;
