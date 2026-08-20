//! TON chain support for x402 payments.
//!
//! - [`TvmChainReference`] — CAIP-2 `-239` / `-3`
//! - [`TvmAddress`] — raw `workchain:hex` (user-friendly forms accepted on parse)
//! - [`TvmTokenDeployment`] — jetton minter metadata
//! - [`TvmChainProvider`] — Highload V3 signer + REST (feature `facilitator`)

/// Core TON chain types (addresses, references, token deployments).
pub mod types;
pub use types::*;

/// TON chain provider implementation.
#[cfg(feature = "facilitator")]
pub mod provider;
#[cfg(feature = "facilitator")]
pub use provider::*;

/// REST client abstraction for TON.
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod rpc;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use rpc::*;
