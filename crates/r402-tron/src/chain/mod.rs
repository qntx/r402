//! Tron chain support for x402 payments.
//!
//! This module provides types and providers for interacting with the Tron
//! blockchain in the x402 protocol. It supports TRC-20 token transfers for
//! payment settlement using the same TIP-712 (EIP-712-compatible) typed-data
//! signing scheme as EVM chains.
//!
//! # Key Types
//!
//! - [`TronChainReference`] - A CAIP-2 chain reference identifying a Tron network
//! - [`TronTokenDeployment`] - Token deployment information including contract address and decimals
//! - [`Address`] - A Tron account address (Base58Check-encoded, `T`-prefixed)
//!
//! # Tron Networks
//!
//! - Mainnet: `tron:0x2b6653dc`
//! - Nile testnet: `tron:0xcd8690dc`

/// Core Tron chain types (addresses, references, token deployments).
pub mod types;
pub use types::*;

/// Solidity ABI bindings for contract calldata construction.
#[cfg(feature = "facilitator")]
pub mod contracts;

/// Tron chain provider implementation (TronGrid HTTP API-backed).
#[cfg(feature = "facilitator")]
pub mod provider;
#[cfg(feature = "facilitator")]
pub use provider::*;
