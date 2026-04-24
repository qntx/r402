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

pub mod chain;
pub mod exact;

mod networks;
pub use exact::Eip155Exact;
#[cfg(feature = "client")]
pub use exact::client::{Eip155ExactClient, Eip155ExactClientBuilder, Permit2Approver};
pub use networks::*;
