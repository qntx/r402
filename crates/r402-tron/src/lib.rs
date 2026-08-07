#![cfg_attr(docsrs, feature(doc_cfg))]

//! Tron chain support for the x402 payment protocol.
//!
//! This crate provides implementations of the x402 payment protocol for Tron
//! chains with the "exact" payment scheme based on EIP-3009
//! `transferWithAuthorization` and SUN.io Permit2 via `x402ExactPermit2Proxy`.
//!
//! # Features
//!
//! - **CAIP-2 Addressing**: Uses CAIP-2 chain IDs (e.g., `tron:0x2b6653dc`) for chain identification
//! - **`Base58Check` Addresses**: Tron-native `T`-prefixed addresses with 0x41 prefix
//! - **TIP-712 Signing**: Byte-compatible with EIP-712, using secp256k1 ECDSA
//! - **EIP-3009 Payments**: Gasless token transfers using `transferWithAuthorization`
//! - **Permit2 Payments**: SUN.io Permit2 fallback for tokens without EIP-3009 (e.g. USDT)
//! - **`TronGrid` API**: HTTP REST API for chain interaction (no JSON-RPC)
//!
//! # Architecture
//!
//! The crate is organized into two modules:
//!
//! - [`chain`] - Core Tron chain types, providers, and contract bindings
//! - [`exact`] - Tron "exact" payment scheme
//!
//! # Feature Flags
//!
//! - `server` - Server-side price tag generation
//! - `client` - Client-side payment signing
//! - `facilitator` - Facilitator-side payment verification and settlement
//! - `telemetry` - `OpenTelemetry` tracing support

// Silence the linter when default features disable every consumer.
#[cfg(not(any(feature = "client", feature = "server", feature = "facilitator")))]
use compact_str as _;
#[cfg(feature = "telemetry")]
use tracing_core as _;

/// Default clock-skew tolerance (seconds) applied to TIP-712 time-window checks.
///
/// Tron produces blocks approximately every 3 seconds; we use 6 seconds
/// (2 blocks) to provide a reasonable grace window for clock drift between
/// the facilitator and the Tron network.
pub const TRON_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS: u64 = 6;

pub mod chain;
pub mod exact;

mod networks;
pub use exact::TronExact;
#[cfg(feature = "client")]
pub use exact::client::{Eip3009SigningParams, Permit2SigningParams, SignerLike, TronExactClient};
pub use networks::*;
