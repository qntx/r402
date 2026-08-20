#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    test,
    allow(
        unknown_lints,
        clippy::unused_async_trait_impl,
        reason = "in-crate mock impls of AFIT traits have no .await"
    )
)]

//! Keeta chain support for the x402 payment protocol.
//!
//! This crate implements the x402 `"exact"` scheme for Keeta: the buyer
//! signs a block containing one `SEND` operation, and a facilitator
//! fee-payer transmits it with `TransmitOptions::with_fee_signer`.
//!
//! # Features
//!
//! - **CAIP-2 Addressing**: `keeta:21378` and `keeta:1413829460`
//! - **Token `SEND`**: exact transfer of a token (USDC by default)
//! - **Sponsored fees**: the facilitator signs the fee block
//! - **In-process facilitator**: `Block::try_from` verify and `UserClient::transmit` settle
//!
//! # Feature Flags
//!
//! - `server` — server-side price tag generation
//! - `client` — client-side block signing
//! - `facilitator` — facilitator-side payment verification and settlement
//! - `telemetry` — `tracing` instrumentation

#[cfg(not(any(feature = "client", feature = "facilitator")))]
use base64 as _;
#[cfg(feature = "telemetry")]
use tracing_core as _;

/// Default USDC decimal precision.
pub const DEFAULT_TOKEN_DECIMALS: u8 = 6;

pub mod chain;
pub mod exact;

mod networks;
#[cfg(feature = "facilitator")]
pub use chain::KeetaChainProvider;
pub use exact::KeetaExact;
#[cfg(feature = "client")]
pub use exact::client::{KeetaExactClient, KeetaSigner};
pub use networks::*;
