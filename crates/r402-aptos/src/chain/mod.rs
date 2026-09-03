//! Aptos chain primitives, BCS payload codec, and facilitator provider.
//!
//! - [`AptosChainReference`] — CAIP-2 `1` / `2`
//! - [`AptosAddress`] — 32-byte hex (`0x` + 64 hex chars)
//! - [`AptosTokenDeployment`] — FA metadata
//! - [`AptosChainProvider`] — facilitator fee payer + `aptos-sdk` (feature `facilitator`)

pub mod account;
pub use account::*;

/// TS x402 payload codec: outer base64 of JSON-of-bytes, inner SDK BCS.
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod codec;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use codec::*;

/// Aptos chain provider implementation.
#[cfg(feature = "facilitator")]
pub mod provider;
#[cfg(feature = "facilitator")]
pub use provider::*;
