//! Hedera chain support for x402 payments.
//!
//! - [`HederaChainReference`] — CAIP-2 `mainnet` / `testnet`
//! - [`HederaAddress`] — entity id (`0.0.n`) or alias
//! - [`HederaTokenDeployment`] — HBAR / HTS token metadata
//! - [`HederaChainProvider`] — facilitator fee payer + Mirror REST (feature `facilitator`)

/// Core Hedera chain types (addresses, references, token deployments).
pub mod account;
pub use account::*;

/// Hedera chain provider implementation.
#[cfg(feature = "facilitator")]
pub mod provider;
#[cfg(feature = "facilitator")]
pub use provider::*;

/// Hiero transaction inspection and client transfer construction.
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod tx;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use tx::*;

/// Mirror Node REST client.
#[cfg(feature = "facilitator")]
pub mod rpc;
#[cfg(feature = "facilitator")]
pub use rpc::*;
