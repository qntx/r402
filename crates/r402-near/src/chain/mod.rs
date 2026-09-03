//! NEAR chain support for x402 payments.
//!
//! - [`NearChainReference`] — CAIP-2 `mainnet` / `testnet`
//! - [`NearAddress`] — named (`alice.testnet`) or implicit (64 lowercase hex)
//! - [`NearTokenDeployment`] — NEP-141 token metadata
//! - [`NearChainProvider`] — facilitator relayer + JSON-RPC (feature `facilitator`)

/// Core NEAR chain types (addresses, references, token deployments).
pub mod account;
pub use account::*;

/// NEAR chain provider implementation.
#[cfg(feature = "facilitator")]
pub mod provider;
#[cfg(feature = "facilitator")]
pub use provider::*;

/// JSON-RPC client abstraction for NEAR.
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod rpc;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use rpc::*;
