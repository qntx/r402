#![cfg_attr(docsrs, feature(doc_auto_cfg))]

//! Solana chain support for the x402 payment protocol.
//!
//! This crate provides implementations of the x402 payment protocol for Solana blockchain.
//! It supports both V1 and V2 protocol versions with the "exact" payment scheme based on
//! SPL Token `transfer` instructions with pre-signed authorization.
//!
//! # Features
//!
//! - **V1 and V2 Protocol Support**: Implements both protocol versions with network name
//!   (V1) and CAIP-2 chain ID (V2) addressing
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
//! - [`exact`] - Solana "exact" payment scheme (V1 + V2)
//!
//! # Feature Flags
//!
//! - `server` - Server-side price tag generation
//! - `client` - Client-side payment signing
//! - `facilitator` - Facilitator-side payment verification and settlement
//! - `telemetry` - `OpenTelemetry` tracing support
//!
pub mod chain;
pub mod exact;

mod networks;
pub use networks::*;

pub use exact::{V1SolanaExact, V2SolanaExact};

#[cfg(feature = "client")]
pub use exact::client::{V1SolanaExactClient, V2SolanaExactClient};
