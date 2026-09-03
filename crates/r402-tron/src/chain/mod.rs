//! Tron chain support for x402 payments.
//!
//! Types and providers for TRC-20 transfers using TIP-712 (EIP-712-compatible)
//! typed-data signing.
//!
//! # Key Types
//!
//! - [`TronChainReference`] — CAIP-2 chain reference (`tron:0x2b6653dc`)
//! - [`TronTokenDeployment`] — contract address, decimals, optional TIP-712 domain
//! - [`Address`] — `Base58Check` `T`-prefixed account
//!
//! # Tron Networks
//!
//! - Mainnet: `tron:0x2b6653dc`
//! - Nile testnet: `tron:0xcd8690dc`

/// Default clock-skew tolerance (seconds) applied to TIP-712 time-window checks.
///
/// Tron produces blocks approximately every 3 seconds; 6 seconds (2 blocks)
/// covers clock drift between the facilitator and the network.
pub const TRON_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS: u64 = 6;

/// Core Tron chain types (addresses, references, token deployments).
pub mod account;
pub use account::*;

/// Solidity ABI bindings for contract calldata construction.
#[cfg(feature = "facilitator")]
#[cfg_attr(docsrs, doc(cfg(feature = "facilitator")))]
pub mod contracts;

/// Tron chain provider implementation (`TronGrid` HTTP API-backed).
#[cfg(feature = "facilitator")]
#[cfg_attr(docsrs, doc(cfg(feature = "facilitator")))]
pub mod provider;
#[cfg(feature = "facilitator")]
pub use provider::*;
