//! EVM chain support for x402 payments via EIP-155.
//!
//! This module provides types and providers for interacting with EVM-compatible blockchains
//! in the x402 protocol. It supports ERC-3009 `transferWithAuthorization` for gasless
//! token transfers, which is the foundation of x402 payments on EVM chains.
//!
//! # Key Types
//!
//! - [`Eip155ChainReference`] - A numeric chain ID for EVM networks (e.g., `8453` for Base)
//! - [`Eip155ChainProvider`] - Provider for interacting with EVM chains
//! - [`Eip155TokenDeployment`] - Token deployment information including address and decimals
//! - [`MetaTransaction`] - Parameters for sending meta-transactions
//!
//! # Submodules
//!
//! - [`types`] - Wire format types like [`ChecksummedAddress`](types::ChecksummedAddress) and [`TokenAmount`](types::TokenAmount)
//! - [`nonce`] - Nonce management for concurrent transaction submission
//!
//! # ERC-3009 Support
//!
//! The x402 protocol uses ERC-3009 `transferWithAuthorization` for payments. This allows
//! users to sign payment authorizations off-chain, which the facilitator then submits
//! on-chain. The facilitator pays the gas fees and is reimbursed through the payment.
pub mod types;

/// Pending nonce management for EVM transactions.
#[cfg(feature = "facilitator")]
pub mod nonce;
/// EVM chain provider implementation.
#[cfg(feature = "facilitator")]
pub mod provider;

#[cfg(feature = "facilitator")]
pub use nonce::*;
#[cfg(feature = "facilitator")]
pub use provider::*;
pub use types::*;
