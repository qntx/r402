//! Buyer-signed payment authorization.

use serde::{Deserialize, Serialize};

use super::{Extensions, ResourceInfo, V2, Version2};

/// Signed payment authorization sent by the buyer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct PaymentPayload<TAccepted, TPayload> {
    /// Terms the buyer accepted.
    pub accepted: TAccepted,
    /// Scheme-specific signed payload.
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
    /// Constructs a payload from the two required fields.
    #[must_use]
    pub fn new(accepted: TAccepted, payload: TPayload) -> Self {
        Self {
            accepted,
            payload,
            resource: None,
            x402_version: V2,
            extensions: Extensions::new(),
        }
    }

    /// Attaches optional resource metadata.
    #[must_use]
    pub fn with_resource(mut self, resource: ResourceInfo) -> Self {
        self.resource = Some(resource);
        self
    }

    /// Passes through an optional resource.
    #[must_use]
    pub fn with_optional_resource(mut self, resource: Option<ResourceInfo>) -> Self {
        self.resource = resource;
        self
    }

    /// Replaces the `extensions` block.
    #[must_use]
    pub fn with_extensions(mut self, extensions: Extensions) -> Self {
        self.extensions = extensions;
        self
    }
}
