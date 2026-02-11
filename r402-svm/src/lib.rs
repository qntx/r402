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
//! # Usage Examples
//!
//! ## Server: Creating a Price Tag
//!
//! ```ignore
//! use r402_svm::{V1SolanaExact, KnownNetworkSolana};
//! use r402::networks::USDC;
//!
//! // Get USDC deployment on Solana mainnet
//! let usdc = USDC::solana();
//!
//! // Create a price tag for 1 USDC
//! let price_tag = V1SolanaExact::price_tag(
//!     "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM",
//!     usdc.amount(1_000_000u64),
//! );
//! ```
//!
//! ## Client: Signing a Payment
//!
//! ```ignore
//! use r402_svm::V1SolanaExactClient;
//! use solana_keypair::Keypair;
//!
//! let keypair = Keypair::new();
//! let client = V1SolanaExactClient::new(keypair);
//!
//! // Use client to sign payment candidates
//! let candidates = client.accept(&payment_required);
//! ```
//!
//! ## Facilitator: Verifying and Settling
//!
//! ```ignore
//! use r402_svm::{V1SolanaExact, SolanaChainProvider};
//! use r402::scheme::X402SchemeFacilitatorBuilder;
//!
//! let provider = SolanaChainProvider::from_config(&config).await?;
//! let facilitator = V1SolanaExact.build(provider, None)?;
//!
//! // Verify payment
//! let verify_response = facilitator.verify(verify_request).await?;
//!
//! // Settle payment
//! let settle_response = facilitator.settle(settle_request).await?;
//! ```

pub mod chain;
pub mod exact;

mod networks;
pub use networks::*;

pub use exact::{V1SolanaExact, V2SolanaExact};

#[cfg(feature = "client")]
pub use exact::client::{V1SolanaExactClient, V2SolanaExactClient};
