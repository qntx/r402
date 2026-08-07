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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
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

impl<TAccepted, TPayload> PaymentPayload<TAccepted, TPayload> {
    /// Constructs a payload from the two required fields. Use the
    /// [`Self::with_resource`] / [`Self::with_extensions`] builders to
    /// attach the optional blocks.
    #[must_use]
    pub fn new(accepted: TAccepted, payload: TPayload) -> Self {
        Self {
            accepted,
            payload,
            resource: None,
            x402_version: super::V2,
            extensions: Extensions::new(),
        }
    }

    /// Builder: attaches optional resource metadata.
    #[must_use]
    pub fn with_resource(mut self, resource: ResourceInfo) -> Self {
        self.resource = Some(resource);
        self
    }

    /// Builder: attaches an optional resource (passes through `None`
    /// untouched, useful when the value is produced via `Option::map`).
    #[must_use]
    pub fn with_optional_resource(mut self, resource: Option<ResourceInfo>) -> Self {
        self.resource = resource;
        self
    }

    /// Builder: replaces the `extensions` block.
    #[must_use]
    pub fn with_extensions(mut self, extensions: Extensions) -> Self {
        self.extensions = extensions;
        self
    }
}
