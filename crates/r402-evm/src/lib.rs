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
//! - [`signer`] / [`permit2`] / [`asset`] / [`signature`] - primitives shared by every scheme
//! - [`exact`] - EIP-155 "exact" payment scheme
//! - [`upto`]  - EIP-155 "upto" payment scheme (Permit2, usage-based)
//! - [`auth_capture`] - authorize / capture / charge (commerce-payments escrow)
//! - [`batch_settlement`] - capital-backed channel vouchers
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

pub mod asset;
pub mod auth_capture;
pub mod batch_settlement;
pub mod chain;
pub mod eip2612;
pub mod erc20_approval;
#[cfg(feature = "facilitator")]
#[cfg_attr(docsrs, doc(cfg(feature = "facilitator")))]
pub mod error;
pub mod exact;
pub mod permit2;
#[cfg(feature = "facilitator")]
#[cfg_attr(docsrs, doc(cfg(feature = "facilitator")))]
pub mod signature;
#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub mod signer;
pub mod upto;

#[cfg(feature = "facilitator")]
pub use error::Eip155ExactError;

mod networks;
pub use asset::{AssetTransferMethod, VALIDATOR_ADDRESS};
pub use auth_capture::Eip155AuthCapture;
#[cfg(feature = "facilitator")]
pub use auth_capture::Eip155AuthCaptureFacilitator;
#[cfg(feature = "client")]
pub use auth_capture::{Eip155AuthCaptureClient, sign_auth_capture};
pub use batch_settlement::Eip155BatchSettlement;
#[cfg(feature = "facilitator")]
pub use batch_settlement::{
    ChannelStore, Eip155BatchSettlementFacilitator, MemoryChannelStore, compute_channel_id,
};
#[cfg(feature = "client")]
pub use batch_settlement::{Eip155BatchSettlementClient, sign_voucher, sign_voucher_payload};
pub use eip2612::{
    EIP2612_GAS_SPONSORING_KEY, EIP2612_GAS_SPONSORING_VERSION, Eip2612ParseError,
    Eip2612SignedPermit,
};
pub use erc20_approval::{
    ERC20_APPROVAL_GAS_SPONSORING_KEY, ERC20_APPROVAL_GAS_SPONSORING_VERSION,
    Erc20ApprovalGasSponsoringInfo, Erc20ApprovalParseError,
};
pub use exact::Eip155Exact;
#[cfg(feature = "client")]
pub use exact::client::{Eip155ExactClient, Eip155ExactClientBuilder};
pub use networks::*;
pub use permit2::{PERMIT2_ADDRESS, Permit2TokenPermissions};
#[cfg(feature = "client")]
pub use permit2::{Permit2Approver, permit2_allowance_calldata, permit2_approval_calldata};
#[cfg(feature = "facilitator")]
pub use signature::StructuredSignatureFormatError;
#[cfg(feature = "client")]
pub use signer::SignerLike;
pub use upto::Eip155Upto;
#[cfg(feature = "client")]
pub use upto::client::{Eip155UptoClient, Eip2612SigningParams, sign_eip2612_permit};
