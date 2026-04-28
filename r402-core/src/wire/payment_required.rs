//! HTTP 402 response body.

use compact_str::CompactString;
use serde::{Deserialize, Serialize};

use super::{Extensions, PaymentRequirements, ResourceInfo, Version2};

/// Body of an HTTP 402 "Payment Required" response.
///
/// Contains:
///
/// - the x402 version marker,
/// - an optional human-readable `error` string for malformed clients,
/// - resource metadata,
/// - the list of [`PaymentRequirements`] the seller will accept, and
/// - an optional `extensions` block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct PaymentRequired {
    /// Protocol version (always `2`).
    pub x402_version: Version2,
    /// Optional error message describing why the request was rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<CompactString>,
    /// Resource metadata.
    pub resource: ResourceInfo,
    /// Accepted payment terms.
    #[serde(default)]
    pub accepts: Vec<PaymentRequirements>,
    /// Optional extension block.
    #[serde(default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl PaymentRequired {
    /// Constructs a 402 body with the resource block and an empty
    /// `accepts` list. Use [`Self::with_accepts`] to attach payment
    /// requirements and [`Self::with_error`] to surface a diagnostic
    /// message to malformed clients.
    #[must_use]
    pub fn new(resource: ResourceInfo) -> Self {
        Self {
            x402_version: super::V2,
            error: None,
            resource,
            accepts: Vec::new(),
            extensions: Extensions::new(),
        }
    }

    /// Builder: replaces the accepted payment requirements list.
    #[must_use]
    pub fn with_accepts(mut self, accepts: Vec<PaymentRequirements>) -> Self {
        self.accepts = accepts;
        self
    }

    /// Builder: appends a single payment requirement to the `accepts` list.
    #[must_use]
    pub fn add_accept(mut self, accept: PaymentRequirements) -> Self {
        self.accepts.push(accept);
        self
    }

    /// Builder: attaches a human-readable error message describing why the
    /// request was rejected (e.g. `"missing X-PAYMENT header"`).
    #[must_use]
    pub fn with_error(mut self, error: impl Into<CompactString>) -> Self {
        self.error = Some(error.into());
        self
    }

    /// Builder: replaces the `extensions` block.
    #[must_use]
    pub fn with_extensions(mut self, extensions: Extensions) -> Self {
        self.extensions = extensions;
        self
    }
}
