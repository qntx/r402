//! Buyer-signed payment authorization.

use serde::{Deserialize, Serialize};

use super::{Extensions, ResourceInfo, Version2};

/// A signed payment authorization sent by the buyer to the seller.
///
/// In x402 v2 the payload is self-describing: it carries the `accepted`
/// requirements the buyer chose (so the facilitator can re-verify them)
/// plus the scheme-specific `payload`, an optional resource descriptor,
/// and an optional `extensions` map.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentPayload<TAccepted, TPayload> {
    /// The terms the buyer accepted (a full [`super::PaymentRequirements`] form).
    pub accepted: TAccepted,
    /// Scheme-specific signed payload (e.g., EIP-3009 authorization).
    pub payload: TPayload,
    /// Optional resource metadata copied from the 402 response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceInfo>,
    /// Protocol version marker (always `2`).
    pub x402_version: Version2,
    /// Optional extension payload block.
    #[serde(default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}
