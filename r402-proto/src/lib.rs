//! Wire format types for the x402 payment protocol.
//!
//! This crate defines the serialization-level data structures used by the
//! x402 protocol, covering both V1 (legacy) and V2 (current) formats.
//! It has minimal dependencies (only `serde` and `serde_json`) and is
//! intended to be the shared "lingua franca" across the entire r402 stack.
//!
//! # Modules
//!
//! - [`v2`] — Current protocol types (`PaymentRequirements`, `PaymentPayload`, etc.)
//! - [`v1`] — Legacy protocol types (`PaymentRequirementsV1`, `PaymentPayloadV1`, etc.)
//! - [`responses`] — Facilitator responses (`VerifyResponse`, `SettleResponse`, etc.)
//! - [`helpers`] — Version detection, parsing, and network pattern matching

pub mod helpers;
pub mod responses;
pub mod v1;
pub mod v2;

pub use responses::{SettleResponse, SupportedKind, SupportedResponse, VerifyResponse};
pub use v1::{PaymentPayloadV1, PaymentRequiredV1, PaymentRequirementsV1, SupportedResponseV1};
pub use v2::{
    PaymentPayload, PaymentRequired, PaymentRequirements, ResourceInfo, SettleRequest,
    VerifyRequest,
};

/// Current protocol version.
pub const X402_VERSION: u32 = 2;

/// CAIP-2 format network identifier (e.g., `"eip155:8453"`, `"solana:mainnet"`).
pub type Network = String;

/// Errors that can occur when parsing x402 protocol messages.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// The `x402Version` field is missing from the JSON data.
    #[error("missing x402Version field")]
    MissingVersion,

    /// The `x402Version` field has an unsupported value.
    #[error("invalid x402Version: {0}")]
    InvalidVersion(u32),

    /// A required field is missing from the JSON data.
    #[error("missing required field: {0}")]
    MissingField(&'static str),

    /// JSON deserialization failed.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}
