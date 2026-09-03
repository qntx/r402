//! Stellar chain primitives, RPC, XDR, and facilitator provider.
//!
//! - [`StellarChainReference`] — CAIP-2 `pubnet` / `testnet`
//! - [`StellarAddress`] — G-account, C-account, or M-account
//! - [`StellarTokenDeployment`] — SEP-41 token metadata
//! - [`StellarChainProvider`] — facilitator signers + RPC (feature `facilitator`)

pub mod account;
pub use account::*;

/// Stellar RPC client abstraction.
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod rpc;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use rpc::*;

/// Ed25519 signer for transactions and auth entries.
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod signer;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use signer::*;

/// XDR helpers for invoke-host-function transfers.
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod xdr;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use xdr::*;

/// Stellar chain provider implementation.
#[cfg(feature = "facilitator")]
pub mod provider;
#[cfg(feature = "facilitator")]
pub use provider::*;
