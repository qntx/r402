//! EVM chain support for x402 payments via EIP-155.
//!
//! Types and providers for ERC-3009 `transferWithAuthorization` on EVM chains.
//!
//! # Key Types
//!
//! - [`Eip155ChainReference`] — numeric chain ID (e.g. `8453` for Base)
//! - [`Eip155ChainProvider`] — RPC + wallet provider
//! - [`Eip155TokenDeployment`] — token address, decimals, EIP-712 domain
//! - [`MetaTransaction`] — target, calldata, confirmations
pub mod account;

/// Default clock-skew tolerance (seconds) for EIP-712 time-window checks.
///
/// Aligned with the Go SDK `DefaultPermit2DeadlineBuffer` (one EVM block ≈ 6 s).
pub const EVM_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS: u64 = 6;

/// Shared Solidity interfaces used by more than one scheme facilitator.
#[cfg(feature = "facilitator")]
pub mod contracts;

/// Pending nonce management for EVM transactions.
#[cfg(feature = "facilitator")]
pub mod nonce;
/// EVM chain provider implementation.
#[cfg(feature = "facilitator")]
pub mod provider;

pub use account::*;
#[cfg(feature = "facilitator")]
pub use nonce::*;
#[cfg(feature = "facilitator")]
pub use provider::*;
