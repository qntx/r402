//! On-the-wire JSON types for the x402 protocol.
//!
//! All types in this module are authoritative for their JSON serialization:
//! field names match the spec exactly (camelCase), unknown fields are rejected,
//! and identifier strings use [`compact_str::CompactString`] to avoid
//! allocations on the hot path.
//!
//! Files follow protocol layers, not one-type-per-file:
//!
//! - **codec** — JSON-safe primitives ([`Base64Bytes`], [`U64String`],
//!   [`UnixTimestamp`], [`Version`])
//! - **extensions** — [`Extensions`] / [`ExtensionEntry`]
//! - **offer** — 402 challenge and buyer payload ([`ResourceInfo`],
//!   [`PaymentRequirements`], [`PaymentRequired`], [`PaymentPayload`],
//!   [`PriceTag`])
//! - **rpc** — facilitator verify / settle / `/supported`

mod codec;
mod extensions;
mod offer;
mod rpc;

pub use codec::{Base64Bytes, U64String, UnixTimestamp, V2, Version, Version2};
pub use extensions::{ExtensionEntry, Extensions};
pub use offer::{
    Enricher, PaymentPayload, PaymentRequired, PaymentRequirements, PriceTag, ResourceInfo,
    find_matching_requirements,
};
pub use rpc::{
    DEFAULT_ASSET_DECIMALS, SettleRequest, SettleResponse, SettlementOverrideError,
    SettlementOverrides, SupportedPaymentKind, SupportedResponse, TypedVerifyRequest,
    VerifyRequest, VerifyResponse, asset_decimals_from_extra, resolve_settlement_override_amount,
};
