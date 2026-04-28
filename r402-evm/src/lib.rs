#![cfg_attr(docsrs, feature(doc_cfg))]

//! EIP-155 (EVM) chain support for the x402 payment protocol.
//!
//! This crate provides implementations of the x402 payment protocol for EVM chains
//! with the "exact" payment scheme based on ERC-3009 `transferWithAuthorization`.
//!
//! # Features
//!
//! - **CAIP-2 Addressing**: Uses CAIP-2 chain IDs (e.g., `eip155:8453`) for chain identification
//! - **ERC-3009 Payments**: Gasless token transfers using `transferWithAuthorization`
//! - **Smart Wallet Support**: EIP-1271 for deployed wallets, EIP-6492 for counterfactual wallets
//! - **Multiple Signers**: Round-robin signer selection for load distribution
//! - **Nonce Management**: Automatic nonce tracking with pending transaction awareness
//!
//! # Architecture
//!
//! The crate is organized into several modules:
//!
//! - [`chain`] - Core EVM chain types, providers, and configuration
//! - [`exact`] - EIP-155 "exact" payment scheme
//! - [`upto`]  - EIP-155 "upto" payment scheme (Permit2, usage-based)
//!
//! # Feature Flags
//!
//! - `server` - Server-side price tag generation
//! - `client` - Client-side payment signing
//! - `facilitator` - Facilitator-side payment verification and settlement
//! - `telemetry` - `OpenTelemetry` tracing support
//!

// Consumed by the feature-gated modules below; silence the linter when
// default features disable every consumer of `compact_str`.
#[cfg(not(any(feature = "client", feature = "server", feature = "facilitator")))]
use compact_str as _;
// Dev-deps consumed by `tests/onchain_deployment.rs` only. The lib test
// target links every dev-dep and would otherwise trip
// `unused_crate_dependencies` because no unit test references them.
#[cfg(test)]
use {alloy_provider as _, alloy_transport_http as _, tokio as _, url as _};

/// Default clock-skew tolerance (seconds) applied to EIP-712 time-window checks.
///
/// Aligned with the Go reference SDK's
/// `DefaultPermit2DeadlineBuffer` (one EVM block ≈ 6 s) so that the same
/// signed payload is accepted by either implementation. A larger value is
/// more lenient toward clock drift; a value of `0` enforces exact-time
/// boundaries (useful for deterministic tests).
pub const EVM_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS: u64 = 6;

pub mod chain;
pub mod exact;
pub mod upto;

mod networks;
pub use exact::Eip155Exact;
#[cfg(feature = "client")]
pub use exact::client::{Eip155ExactClient, Eip155ExactClientBuilder, Permit2Approver};
pub use networks::*;
pub use upto::Eip155Upto;
#[cfg(feature = "client")]
pub use upto::client::Eip155UptoClient;
