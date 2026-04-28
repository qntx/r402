#![cfg_attr(docsrs, feature(doc_cfg))]

//! Solana chain support for the x402 payment protocol.
//!
//! This crate provides implementations of the x402 payment protocol for Solana blockchain
//! with the "exact" payment scheme based on SPL Token `transfer` instructions with
//! pre-signed authorization.
//!
//! # Features
//!
//! - **CAIP-2 Addressing**: Uses CAIP-2 chain IDs for chain identification
//! - **SPL Token Payments**: Token transfers using pre-signed transaction authorization
//! - **Compute Budget Management**: Automatic compute unit limit and price configuration
//! - **`WebSocket` Support**: Optional pubsub for faster transaction confirmation
//! - **Balance Verification**: On-chain balance checks before settlement
//!
//! # Architecture
//!
//! The crate is organized into several modules:
//!
//! - [`chain`] - Core Solana chain types, providers, and configuration
//! - [`exact`] - Solana "exact" payment scheme
//!
//! # Feature Flags
//!
//! - `server` - Server-side price tag generation
//! - `client` - Client-side payment signing
//! - `facilitator` - Facilitator-side payment verification and settlement
//! - `telemetry` - `OpenTelemetry` tracing support
//!

// Consumed by feature-gated modules; quiet the linter when default features
// disable every consumer.
#[cfg(not(feature = "facilitator"))]
use base64 as _;
#[cfg(not(feature = "facilitator"))]
use futures_util as _;

/// Recommended hard cap for `max_compute_unit_price` (micro-lamports).
///
/// Aligned with the Go reference SDK
/// (`mechanisms/svm/exact/facilitator/scheme.go: maxComputeUnitPriceMicroLamports`)
/// so that any client transaction passing Go's verifier passes r402's, and
/// vice versa. A value of 5 000 000 micro-lamports per CU corresponds to
/// roughly 0.005 SOL per million CU, well above realistic priority-fee
/// surge prices on Solana mainnet, while bounding the worst-case fee a
/// fee-paying facilitator can be tricked into paying.
pub const SOLANA_MAX_COMPUTE_UNIT_PRICE_MICROLAMPORTS: u64 = 5_000_000;

pub mod chain;
pub mod exact;

#[cfg(feature = "facilitator")]
#[cfg_attr(docsrs, doc(cfg(feature = "facilitator")))]
pub mod settlement_cache;

mod networks;
pub use exact::SolanaExact;
#[cfg(feature = "client")]
pub use exact::client::SolanaExactClient;
pub use networks::*;
