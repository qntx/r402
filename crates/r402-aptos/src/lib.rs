#![cfg_attr(docsrs, feature(doc_cfg))]

//! Aptos chain support for the x402 payment protocol.
//!
//! This crate implements the x402 `"exact"` scheme for Aptos: the buyer
//! signs a `0x1::primary_fungible_store::transfer` of a fungible asset, and
//! a facilitator fee payer submits it after `aptos-sdk` 0.6 preflight.
//!
//! # Features
//!
//! - **CAIP-2 Addressing**: `aptos:1` (mainnet) and `aptos:2` (testnet)
//! - **Fungible Asset payments**: exact `primary_fungible_store::transfer`
//! - **Fee-payer sponsorship**: `extra.feePayer` is copied from `/supported`
//! - **In-process facilitator**: verify and settle via `aptos-sdk` 0.6.0
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

#[cfg(test)]
mod unused_dev_deps {
    #[cfg(not(any(feature = "client", feature = "facilitator")))]
    use aptos_sdk as _;
    #[cfg(not(any(feature = "client", feature = "facilitator")))]
    use tokio as _;
}

/// Default USDC decimal precision.
pub const DEFAULT_TOKEN_DECIMALS: u8 = 6;

/// Sponsored-transaction cap on `max_gas_amount` (`@x402/aptos` `MAX_GAS_AMOUNT`).
pub const MAX_GAS_AMOUNT: u64 = 500_000;

/// Sponsored-transaction cap on `gas_unit_price` octas (`MAX_GAS_UNIT_PRICE`).
pub const MAX_GAS_UNIT_PRICE: u64 = 1_000;

/// Client `max_gas_amount` — Aptos SDK simple-tx default, under [`MAX_GAS_AMOUNT`].
pub const DEFAULT_CLIENT_MAX_GAS: u64 = 200_000;

/// Client `gas_unit_price` when estimate is unavailable.
pub const DEFAULT_GAS_UNIT_PRICE: u64 = 100;

/// Verify rejects expirations closer than this many seconds.
pub const EXPIRATION_BUFFER_SECONDS: u64 = 5;

/// `0x1::primary_fungible_store::transfer` (client-built path).
pub const PRIMARY_FUNGIBLE_STORE_TRANSFER: &str = "0x1::primary_fungible_store::transfer";

/// `0x1::fungible_asset::transfer` (also accepted at verify).
pub const FUNGIBLE_ASSET_TRANSFER: &str = "0x1::fungible_asset::transfer";

/// Type argument on both transfer entry functions.
pub const FUNGIBLE_ASSET_METADATA: &str = "0x1::fungible_asset::Metadata";

pub mod chain;
pub mod exact;

mod networks;
#[cfg(feature = "facilitator")]
pub use chain::{AptosChainProvider, AptosFeePayer};
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::{
    DecodedAptosPayload, SimpleTransaction, decode_aptos_payload, encode_aptos_payload,
};
pub use exact::AptosExact;
#[cfg(feature = "client")]
pub use exact::client::{AptosExactClient, AptosSigner};
#[cfg(feature = "facilitator")]
pub use exact::facilitator::{AptosExactFacilitator, AptosExactFacilitatorConfig};
pub use networks::*;
