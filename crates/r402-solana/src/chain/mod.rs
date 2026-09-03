//! Solana chain primitives, RPC, and facilitator provider.
//!
//! # Key Types
//!
//! - [`SolanaChainReference`] - A 32-character genesis hash identifying a Solana network
//! - [`SolanaChainProvider`] - Provider for interacting with Solana chains
//! - [`SolanaTokenDeployment`] - Token deployment information including mint, decimals, and token program
//! - [`Address`] - A Solana public key (base58-encoded)
//!
//! # Solana Networks
//!
//! Solana networks are identified by the first 32 characters of their genesis block hash:
//! - Mainnet: `5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp`
//! - Devnet: `EtWTRABZaYq6iMfeYKouRu166VU2xqa1`
//! - Testnet: `4uhcVJyU9pJkvQyS88uRDiswHXSCkY3z`

pub mod account;
pub use account::*;

/// Recommended hard cap for `max_compute_unit_price` (micro-lamports).
///
/// Aligned with the Go reference SDK
/// (`mechanisms/svm/exact/facilitator/scheme.go: maxComputeUnitPriceMicroLamports`).
/// 5 000 000 µLAM/CU is roughly 0.005 SOL per million CU.
pub const SOLANA_MAX_COMPUTE_UNIT_PRICE_MICROLAMPORTS: u64 = 5_000_000;

/// Solana chain provider implementation.
#[cfg(feature = "facilitator")]
pub mod provider;
#[cfg(feature = "facilitator")]
pub use provider::*;

/// RPC client abstraction for Solana.
#[cfg(feature = "client")]
pub mod rpc;
