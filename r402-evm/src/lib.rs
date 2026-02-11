#![cfg_attr(docsrs, feature(doc_auto_cfg))]

//! EIP-155 (EVM) chain support for the x402 payment protocol.
//!
//! This crate provides implementations of the x402 payment protocol for EVM-compatible
//! blockchains using the EIP-155 chain ID standard. It supports both V1 and V2 protocol
//! versions with the "exact" payment scheme based on ERC-3009 `transferWithAuthorization`.
//!
//! # Features
//!
//! - **V1 and V2 Protocol Support**: Implements both protocol versions with network name
//!   (V1) and CAIP-2 chain ID (V2) addressing
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
//! - [`exact`] - EIP-155 "exact" payment scheme (V1 + V2)
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

pub use exact::{V1Eip155Exact, V2Eip155Exact};

#[cfg(feature = "client")]
pub use exact::client::{V1Eip155ExactClient, V2Eip155ExactClient};
