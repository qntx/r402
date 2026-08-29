//! `agent-identity` extension — bind an ERC-8004 agent id to a payment.
//!
//! As x402 moves from "a wallet pays" to "an *agent* pays", buyers and
//! sellers need a stable, verifiable agent identity attached to each
//! payment so treasuries, escrows, and compliance hooks can attribute
//! spend to a specific agent rather than a shared wallet.
//!
//! This extension carries an [ERC-8004] agent id inside the extension
//! `info` object, so it survives facilitator verify/settle without extra
//! round-trips and composes with any scheme (`exact`, `upto`, ...).
//!
//! **Advertise (`PaymentRequired`):**
//! ```json
//! { "info": { "required": false }, "schema": { ... } }
//! ```
//!
//! **Payload (`PaymentPayload`):**
//! ```json
//! { "info": { "required": false, "agentId": "eip155:1:0x…" } }
//! ```
//!
//! The agent id is an opaque string from this extension's point of view —
//! chain-agnostic by design (the ERC-8004 registry is EVM-first, but the
//! binding is a plain field). Validation is deliberately minimal: length
//! bound and no control characters / whitespace, so any URI-style agent
//! identifier fits.
//!
//! [ERC-8004]: https://eips.ethereum.org/EIPS/eip-8004

#![cfg(feature = "ext-agent-identity")]
#![cfg_attr(docsrs, doc(cfg(feature = "ext-agent-identity")))]

use std::future::Future;

use serde_json::{Value, json};
use thiserror::Error;

use super::{AdvertiseContext, Extension, SettleContext, VerifyContext};
use crate::wire::ExtensionEntry;

/// Stable extension key on the wire.
pub const AGENT_IDENTITY_KEY: &str = "agent-identity";

/// Default upper bound on agent-id length.
pub const DEFAULT_MAX_LEN: usize = 200;

/// Official JSON Schema fragment for the extension `info` object.
fn info_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "required": { "type": "boolean" },
            "agentId": {
                "type": "string",
                "minLength": 1,
                "description": "ERC-8004 agent identity bound to this payment"
            }
        },
        "required": ["required"]
    })
}

/// Configuration for [`AgentIdentityExtension`].
#[derive(Debug, Clone, Copy)]
pub struct AgentIdentityConfig {
    /// Whether clients **must** supply an `agentId` (advertised as
    /// `info.required`).
    pub required: bool,
    /// Maximum accepted `agentId` length.
    pub max_len: usize,
}

impl Default for AgentIdentityConfig {
    fn default() -> Self {
        Self {
            required: false,
            max_len: DEFAULT_MAX_LEN,
        }
    }
}

/// Server-side implementation of the `agent-identity` extension.
///
/// Attaches an ERC-8004 agent id to `PaymentRequired` advertisements and
/// surfaces the client-supplied id at verify and settle, so downstream
/// treasuries and compliance hooks can attribute and gate spend per agent.
#[derive(Debug, Clone, Copy)]
pub struct AgentIdentityExtension {
    config: AgentIdentityConfig,
}

impl AgentIdentityExtension {
    /// Constructs an extension with default configuration (`required: false`).
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(AgentIdentityConfig::default())
    }

    /// Constructs an extension with the given configuration.
    #[must_use]
    pub const fn with_config(config: AgentIdentityConfig) -> Self {
        Self { config }
    }

    /// Builder: require clients to supply an agent id.
    #[must_use]
    pub const fn with_required(mut self, required: bool) -> Self {
        self.config.required = required;
        self
    }

    /// Whether an agent id is required from clients.
    #[must_use]
    pub const fn is_required(&self) -> bool {
        self.config.required
    }

    /// Validates a payload extension entry against this server's policy.
    ///
    /// Returns the extracted agent id when present and well-formed.
    ///
    /// # Errors
    ///
    /// - [`AgentIdentityError::IdentityRequired`] when `required` and no id
    /// - [`AgentIdentityError::InvalidAgentId`] when format rules fail
    pub fn validate_payload_entry(
        &self,
        entry: Option<&ExtensionEntry>,
    ) -> Result<Option<String>, AgentIdentityError> {
        let owned = entry.map(ExtensionEntry::to_value);
        let id = owned.as_ref().and_then(extract_agent_id).map(str::to_owned);
        match id {
            None if self.config.required => Err(AgentIdentityError::IdentityRequired),
            None => Ok(None),
            Some(raw) => {
                validate_agent_id(&raw, self.config.max_len)?;
                Ok(Some(raw))
            }
        }
    }
}

impl Default for AgentIdentityExtension {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors from agent-identity validation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum AgentIdentityError {
    /// Server set `required: true` but the client omitted `agentId`.
    #[error("agent-identity required but missing")]
    IdentityRequired,
    /// Client `agentId` fails length or charset rules.
    #[error("invalid agent id: {0}")]
    InvalidAgentId(String),
}

/// Validates an agent id: non-empty, at most `max_len` chars, no control
/// characters or whitespace.
///
/// # Errors
///
/// [`AgentIdentityError::InvalidAgentId`] when length or charset rules fail.
pub fn validate_agent_id(id: &str, max_len: usize) -> Result<(), AgentIdentityError> {
    if id.is_empty() {
        return Err(AgentIdentityError::InvalidAgentId(
            "must not be empty".into(),
        ));
    }
    let len = id.chars().count();
    if len > max_len {
        return Err(AgentIdentityError::InvalidAgentId(format!(
            "length {len} exceeds {max_len}"
        )));
    }
    if id.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(AgentIdentityError::InvalidAgentId(
            "must not contain whitespace or control characters".into(),
        ));
    }
    Ok(())
}

/// Extracts the agent id from a payload extension value.
///
/// Accepts the canonical `info.agentId`, bare `agentId` / `agent_id`
/// (structured or raw entry values), and bare `id` for symmetry with
/// identifier-style extensions.
fn extract_agent_id(value: &Value) -> Option<&str> {
    value
        .get("info")
        .and_then(|info| {
            info.get("agentId")
                .or_else(|| info.get("agent_id"))
                .or_else(|| info.get("id"))
        })
        .or_else(|| value.get("agentId"))
        .or_else(|| value.get("agent_id"))
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
}

impl Extension for AgentIdentityExtension {
    fn id(&self) -> &'static str {
        AGENT_IDENTITY_KEY
    }

    fn advertise(&self, _ctx: &AdvertiseContext<'_>) -> Option<ExtensionEntry> {
        Some(ExtensionEntry::with_schema(
            json!({ "required": self.config.required }),
            info_schema(),
        ))
    }

    fn on_verify<'a>(
        &'a self,
        ctx: &VerifyContext<'_>,
    ) -> impl Future<Output = Option<ExtensionEntry>> + Send + 'a {
        let entry = ctx.payload.extensions.get(AGENT_IDENTITY_KEY);
        let result = match self.validate_payload_entry(entry) {
            Err(AgentIdentityError::IdentityRequired) => Some(ExtensionEntry::info(json!({
                "required": true,
                "status": "missing_required",
            }))),
            Err(AgentIdentityError::InvalidAgentId(msg)) => Some(ExtensionEntry::info(json!({
                "required": self.config.required,
                "status": "invalid",
                "message": msg,
            }))),
            Ok(None) => None,
            Ok(Some(agent_id)) => Some(ExtensionEntry::info(json!({
                "required": self.config.required,
                "agentId": agent_id,
                "status": "accepted",
            }))),
        };
        std::future::ready(result)
    }

    fn on_settle<'a>(
        &'a self,
        ctx: &SettleContext<'_>,
    ) -> impl Future<Output = Option<ExtensionEntry>> + Send + 'a {
        let entry = ctx.payload.extensions.get(AGENT_IDENTITY_KEY);
        let agent_id = entry
            .map(ExtensionEntry::to_value)
            .as_ref()
            .and_then(extract_agent_id)
            .map(str::to_owned);
        let result = agent_id.map(|agent_id| {
            ExtensionEntry::info(json!({
                "required": self.config.required,
                "agentId": agent_id,
                "status": "settled",
            }))
        });
        std::future::ready(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn advertise_matches_official_shape() {
        let ext = AgentIdentityExtension::new().with_required(true);
        let entry = ext
            .advertise(&AdvertiseContext { requirement: None })
            .unwrap();
        let value = entry.to_value();
        assert_eq!(value["info"]["required"], true);
        assert!(value.get("schema").is_some());
        assert!(value["schema"]["properties"].get("agentId").is_some());
    }

    #[test]
    fn extract_agent_id_from_info_envelope() {
        let v = json!({"info": {"required": false, "agentId": "eip155:1:0xabc"}});
        assert_eq!(extract_agent_id(&v), Some("eip155:1:0xabc"));
    }

    #[test]
    fn extract_agent_id_from_bare_object() {
        let v = json!({"agentId": "eip155:1:0xabc"});
        assert_eq!(extract_agent_id(&v), Some("eip155:1:0xabc"));
    }

    #[test]
    fn extract_agent_id_from_snake_case() {
        let v = json!({"agent_id": "eip155:1:0xabc"});
        assert_eq!(extract_agent_id(&v), Some("eip155:1:0xabc"));
    }

    #[test]
    fn extract_agent_id_missing() {
        let v = json!({"info": {"required": false}});
        assert_eq!(extract_agent_id(&v), None);
    }

    #[test]
    fn validate_agent_id_rules() {
        assert!(validate_agent_id("eip155:1:0xabc", DEFAULT_MAX_LEN).is_ok());
        assert!(validate_agent_id("urn:erc8004:agent:42", DEFAULT_MAX_LEN).is_ok());
        assert!(validate_agent_id("", DEFAULT_MAX_LEN).is_err());
        assert!(validate_agent_id("has space", DEFAULT_MAX_LEN).is_err());
        assert!(validate_agent_id("tab\tseparated", DEFAULT_MAX_LEN).is_err());
        let long = "x".repeat(201);
        assert!(validate_agent_id(&long, DEFAULT_MAX_LEN).is_err());
    }

    #[test]
    fn validate_required_missing() {
        let ext = AgentIdentityExtension::new().with_required(true);
        assert!(matches!(
            ext.validate_payload_entry(None),
            Err(AgentIdentityError::IdentityRequired)
        ));
    }

    #[test]
    fn validate_optional_missing_ok() {
        let ext = AgentIdentityExtension::new();
        assert_eq!(ext.validate_payload_entry(None).unwrap(), None);
    }

    #[test]
    fn validate_payload_entry_accepts_canonical_id() {
        let ext = AgentIdentityExtension::new();
        let entry = ExtensionEntry::info(json!({
            "required": false,
            "agentId": "eip155:1:0xabc"
        }));
        assert_eq!(
            ext.validate_payload_entry(Some(&entry)).unwrap(),
            Some("eip155:1:0xabc".to_owned())
        );
    }

    #[test]
    fn validate_payload_entry_rejects_bad_id() {
        let ext = AgentIdentityExtension::new();
        let entry = ExtensionEntry::info(json!({ "agentId": "bad id with spaces" }));
        assert!(matches!(
            ext.validate_payload_entry(Some(&entry)),
            Err(AgentIdentityError::InvalidAgentId(_))
        ));
    }
}
