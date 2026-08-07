//! On-the-wire JSON types for the x402 protocol.
//!
//! All types in this module are authoritative for their JSON serialization:
//! field names match the spec exactly (camelCase), unknown fields are rejected,
//! and identifier strings use [`compact_str::CompactString`] to avoid
//! allocations on the hot path.
//!
//! # Layout
//!
//! - [`Base64Bytes`] — raw bytes with base64 encode/decode helpers
//! - [`UnixTimestamp`] — stringified Unix seconds
//! - [`U64String`] — stringified `u64` for JSON-safe integer transport
//! - [`Version<N>`] — protocol-version marker (`N = 2`)
//! - [`Extensions`] / [`ExtensionEntry`] — canonical extension envelope
//! - [`ResourceInfo`] — resource metadata (`description` / `mimeType` optional)
//! - [`PaymentRequirements`] — seller-declared terms
//! - [`PaymentRequired`] — HTTP 402 body
//! - [`PaymentPayload`] — signed buyer authorization (includes accepted terms)
//! - [`VerifyRequest`] / [`VerifyResponse`]
//! - [`SettleRequest`] / [`SettleResponse`] — Success now carries an `amount`
//! - [`SupportedResponse`] / [`SupportedPaymentKind`]
//! - [`PriceTag`] — seller-side builder with enrichment hook

mod base64;
mod extensions;
mod payment_payload;
mod payment_required;
mod payment_requirements;
mod price_tag;
mod request;
mod resource_info;
mod response;
mod supported;
mod timestamp;
mod u64_string;
mod version;

pub use base64::Base64Bytes;
pub use extensions::{ExtensionEntry, Extensions};
pub use payment_payload::PaymentPayload;
pub use payment_required::PaymentRequired;
pub use payment_requirements::PaymentRequirements;
pub use price_tag::{Enricher, PriceTag};
pub use request::{SettleRequest, TypedVerifyRequest, VerifyRequest};
pub use resource_info::ResourceInfo;
pub use response::{SettleResponse, VerifyResponse};
pub use supported::{SupportedPaymentKind, SupportedResponse};
pub use timestamp::UnixTimestamp;
pub use u64_string::U64String;
pub use version::{V2, Version, Version2};
