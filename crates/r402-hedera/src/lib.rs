#![cfg_attr(docsrs, feature(doc_cfg))]

//! Hedera chain support for the x402 payment protocol.
//!
//! This crate implements the x402 `"exact"` scheme for Hedera: the buyer
//! signs a `TransferTransaction` that credits `payTo`, and a facilitator
//! fee payer submits it after Mirror Node preflight.
//!
//! # Features
//!
//! - **CAIP-2 Addressing**: `hedera:mainnet` and `hedera:testnet`
//! - **HBAR and HTS**: native tinybars (`0.0.0`) and USDC token ids
//! - **Fee-payer sponsorship**: `extra.feePayer` is copied from `/supported`
//! - **In-process facilitator**: verify and settle via Hiero SDK
//!
//! # Feature Flags
//!
//! - `server` — server-side price tag generation
//! - `client` — client-side transfer signing
//! - `facilitator` — facilitator-side payment verification and settlement
//! - `telemetry` — `tracing` instrumentation

#[cfg(not(any(feature = "client", feature = "facilitator")))]
use base64 as _;
#[cfg(feature = "telemetry")]
use tracing_core as _;

pub mod chain;
pub mod exact;

mod networks;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::inspect_hedera_transaction;
#[cfg(feature = "facilitator")]
pub use chain::{HederaChainProvider, HederaFeePayer, HederaMirrorClient};
pub use exact::HederaExact;
#[cfg(feature = "client")]
pub use exact::client::{HederaExactClient, HederaSigner};
#[cfg(feature = "facilitator")]
pub use exact::facilitator::{AliasPolicy, HederaExactFacilitator, HederaExactFacilitatorConfig};
pub use networks::*;
