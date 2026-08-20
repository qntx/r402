#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    test,
    allow(
        clippy::unused_async_trait_impl,
        reason = "in-crate mock impls of AFIT traits have no .await"
    )
)]

//! XRPL chain support for the x402 payment protocol.
//!
//! This crate implements the x402 `"exact"` scheme for the XRP Ledger: the
//! buyer signs a `Payment` transaction, and a facilitator submits the signed
//! blob. The payer pays the network fee; facilitators do not sponsor fees.
//!
//! # Features
//!
//! - **CAIP-2 Addressing**: `xrpl:0` (mainnet), `xrpl:1` (testnet), `xrpl:2` (devnet)
//! - **XRP and IOU payments**: native drops or issued-currency decimal values
//! - **Sequence or ticket sequencing**: `extra.assetTransferMethod`
//! - **In-process facilitator**: JSON-RPC verify and settle
//!
//! # Feature Flags
//!
//! - `server` — server-side price tag generation
//! - `client` — client-side payment signing
//! - `facilitator` — facilitator-side payment verification and settlement
//! - `telemetry` — `tracing` instrumentation

#[cfg(all(test, not(any(feature = "client", feature = "facilitator"))))]
use hex as _;
#[cfg(all(test, not(any(feature = "client", feature = "facilitator"))))]
use sha2 as _;
#[cfg(all(test, not(any(feature = "client", feature = "facilitator"))))]
use tokio as _;
#[cfg(feature = "telemetry")]
use tracing_core as _;
#[cfg(all(test, not(any(feature = "client", feature = "facilitator"))))]
use wiremock as _;

/// Default maximum transaction fee accepted by the facilitator, in drops.
pub const DEFAULT_MAX_FEE_DROPS: u64 = 10_000;

/// Default ledger close interval used to map `maxTimeoutSeconds` to ledgers.
pub const DEFAULT_LEDGER_CLOSE_SECONDS: u64 = 5;

/// Extra ledgers added beyond the timeout conversion.
pub const DEFAULT_LEDGER_TOLERANCE: u64 = 2;

/// Partial-payment transaction flag (`tfPartialPayment`).
pub const TF_PARTIAL_PAYMENT: u32 = 0x0002_0000;

/// Account-root flag disabling the master key pair (`lsfDisableMaster`).
pub const LSF_DISABLE_MASTER: u32 = 0x0010_0000;

/// Maximum destination tag (`u32::MAX`).
pub const MAX_DESTINATION_TAG: u32 = u32::MAX;

/// Maximum outstanding tickets an account can hold.
pub const MAX_ACCOUNT_TICKETS: u32 = 250;

/// Settlement cache TTL floor in milliseconds.
pub const SETTLEMENT_TTL_MS: u64 = 120_000;

/// Length of a canonical compressed secp256k1 or ed25519 signing public key.
pub const CANONICAL_SIGNING_PUB_KEY_LEN: usize = 66;

/// SHA-512Half prefix for a signed transaction hash (`TXN\0`).
pub const TRANSACTION_HASH_PREFIX: [u8; 4] = [0x54, 0x58, 0x4E, 0x00];

/// Native XRP decimal precision (drops).
pub const XRP_DECIMALS: u8 = 6;

/// Default RLUSD decimal precision.
pub const RLUSD_DECIMALS: u8 = 15;

/// RLUSD currency as 160-bit hex (ASCII `"RLUSD"` padded).
pub const RLUSD_CURRENCY: &str = "524C555344000000000000000000000000000000";

/// Official Ripple RLUSD issuer on XRPL mainnet.
pub const RLUSD_MAINNET_ISSUER: &str = "rMxCKbEDwqr76QuheSUMdEGf4B9xJ8m5De";

/// Official Ripple RLUSD issuer on XRPL testnet.
pub const RLUSD_TESTNET_ISSUER: &str = "rQhWct2fv4Vc4KRjRgMrxa8xPN9Zx9iLKV";

pub mod chain;
pub mod exact;

mod networks;
#[cfg(feature = "facilitator")]
pub use chain::XrplChainProvider;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use chain::XrplJsonRpc;
pub use exact::XrplExact;
#[cfg(feature = "client")]
pub use exact::client::{XrplExactClient, XrplSigner};
pub use networks::*;
