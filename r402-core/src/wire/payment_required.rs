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
