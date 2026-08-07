//! Tron "exact" payment scheme implementation.
//!
//! This module implements the "exact" payment scheme for Tron, reusing the
//! EIP-3009 `transferWithAuthorization` and Permit2 mechanisms defined by
//! the x402 EVM exact scheme. Tron's TIP-712 typed-data signing is
//! byte-compatible with EIP-712, so the same domain separator and
//! struct-hash algorithm apply; only two things differ from `r402-evm`:
//!
//! - **EOA-only signatures**: Tron has no contract wallets, so neither
//!   EIP-1271 nor EIP-6492 apply — every signature recovers directly to an
//!   externally-owned account.
//! - **`Base58Check` wire addresses**: `payTo` / `asset` / authorization
//!   addresses are Base58Check-encoded ([`crate::chain::Address`]) on the
//!   wire, converted to the raw EVM hex form only at the TIP-712 signing
//!   boundary.
//!
//! # Feature Flags
//!
//! - `server` — Server-side price tag generation
//! - `client` — Client-side payment signing
//! - `facilitator` — Facilitator-side payment verification and settlement
//! - `telemetry` — `OpenTelemetry` tracing support

use r402_core::scheme::SchemeId;

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "facilitator")]
pub mod facilitator;

#[cfg(feature = "client")]
pub mod client;

pub mod error;
pub use error::*;

pub mod types;
pub use types::*;

/// Tron exact scheme identifier.
///
/// Uses CAIP-2 chain IDs (e.g. `tron:0x2b6653dc`) for chain identification
/// and embeds requirements directly in the payload.
#[derive(Debug, Clone, Copy)]
pub struct TronExact;

impl SchemeId for TronExact {
    fn namespace(&self) -> &'static str {
        "tron"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}
