//! Facilitator capability discovery.

use std::collections::HashMap;

use compact_str::CompactString;
use serde::{Deserialize, Serialize};
use serde_with::{VecSkipError, serde_as};

use crate::chain::ChainId;

/// A single payment kind advertised by a facilitator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SupportedPaymentKind {
    /// x402 protocol version (`2`).
    pub x402_version: u8,
    /// Scheme name (e.g. `"exact"`, `"upto"`).
    pub scheme: CompactString,
    /// CAIP-2 network identifier.
    pub network: CompactString,
    /// Optional scheme-specific extras (fee payer, memo, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

/// Response body of a facilitator's `/supported` endpoint.
///
/// Describes the full set of capabilities: payment kinds, known extensions,
/// and signer addresses keyed by CAIP-2 chain pattern.
#[serde_as]
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SupportedResponse {
    /// Supported payment kinds. Invalid entries are silently skipped.
    #[serde_as(as = "VecSkipError<_>")]
    pub kinds: Vec<SupportedPaymentKind>,
    /// Supported extension identifiers.
    #[serde(default)]
    pub extensions: Vec<CompactString>,
    /// Signer addresses indexed by CAIP-2 pattern
    /// (`"eip155:8453"`, `"solana:*"`, ...).
    #[serde(default)]
    pub signers: HashMap<CompactString, Vec<CompactString>>,
}

impl SupportedResponse {
    /// Returns all signer addresses that match the given chain.
    ///
    /// Matches both the exact pattern (`"eip155:8453"`) and the namespace
    /// wildcard (`"eip155:*"`).
    #[must_use]
    pub fn signers_for_chain(&self, chain_id: &ChainId) -> Vec<&str> {
        let exact = CompactString::from(chain_id.to_string());
        let wildcard = CompactString::from(format!("{}:*", chain_id.namespace()));
        let mut out = Vec::new();
        if let Some(list) = self.signers.get(&exact) {
            out.extend(list.iter().map(CompactString::as_str));
        }
        if let Some(list) = self.signers.get(&wildcard) {
            out.extend(list.iter().map(CompactString::as_str));
        }
        out
    }
}
