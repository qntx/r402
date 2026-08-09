//! `payment-identifier` extension — client-supplied idempotency keys.
//!
//! Wire format matches the official x402 v2 extension
//! (`specs/extensions/payment_identifier.md`):
//!
//! **Advertise (`PaymentRequired`):**
//! ```json
//! { "info": { "required": false }, "schema": { ... } }
//! ```
//!
//! **Payload (`PaymentPayload`):**
//! ```json
//! { "info": { "required": false, "id": "pay_…" }, "schema": { ... } }
//! ```
//!
//! Local TTL dedup is an implementation detail and is **not** advertised on
//! the wire.

#![cfg(feature = "ext-payment-id")]
#![cfg_attr(docsrs, doc(cfg(feature = "ext-payment-id")))]

use std::future::Future;
use std::time::Duration;

use serde_json::{Value, json};

use super::{AdvertiseContext, Extension, SettleContext, VerifyContext};
use crate::cache::{Duplicate, TtlSet};
use crate::wire::ExtensionEntry;

/// Stable extension key on the wire.
pub const PAYMENT_IDENTIFIER_KEY: &str = "payment-identifier";

/// Official JSON Schema fragment for the extension `info` object.
fn info_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "required": { "type": "boolean" },
            "id": { "type": "string", "minLength": 16, "maxLength": 128 }
        },
        "required": ["required"]
    })
}

/// Default TTL for remembered payment identifiers (implementation detail).
pub const DEFAULT_MAX_AGE: Duration = Duration::from_mins(10);
/// Default upper bound on tracked identifiers.
pub const DEFAULT_CAPACITY: u64 = 10_000;

/// Minimum `id` length per official spec.
pub const ID_MIN_LEN: usize = 16;
/// Maximum `id` length per official spec.
pub const ID_MAX_LEN: usize = 128;

/// Configuration for [`PaymentIdentifierExtension`].
#[derive(Debug, Clone, Copy)]
pub struct PaymentIdentifierConfig {
    /// Whether clients **must** supply an `id` (advertised as `info.required`).
    pub required: bool,
    /// How long to remember each identifier (local dedup; not on the wire).
    pub max_age: Duration,
    /// Maximum number of distinct identifiers tracked at once.
    pub capacity: u64,
}

impl Default for PaymentIdentifierConfig {
    fn default() -> Self {
        Self {
            required: false,
            max_age: DEFAULT_MAX_AGE,
            capacity: DEFAULT_CAPACITY,
        }
    }
}

/// Server-side implementation of the `payment-identifier` extension.
#[derive(Debug, Clone)]
pub struct PaymentIdentifierExtension {
    config: PaymentIdentifierConfig,
    cache: TtlSet,
}

impl PaymentIdentifierExtension {
    /// Constructs an extension with default configuration (`required: false`).
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(PaymentIdentifierConfig::default())
    }

    /// Constructs an extension with the given configuration.
    #[must_use]
    pub fn with_config(config: PaymentIdentifierConfig) -> Self {
        let cache = TtlSet::new(config.max_age, config.capacity);
        Self { config, cache }
    }

    /// Builder: require clients to supply an identifier.
    #[must_use]
    pub const fn with_required(mut self, required: bool) -> Self {
        self.config.required = required;
        self
    }

    /// Whether an identifier is required from clients.
    #[must_use]
    pub const fn is_required(&self) -> bool {
        self.config.required
    }

    /// Records the identifier and returns whether it was newly seen.
    pub fn record(&self, id: &str) -> Duplicate {
        self.cache.reserve(id)
    }

    /// Returns the backing TTL set (primarily for inspection in tests).
    #[must_use]
    pub const fn cache(&self) -> &TtlSet {
        &self.cache
    }

    /// Validates a payload extension entry against this server's policy.
    ///
    /// # Errors
    ///
    /// - [`PaymentIdError::MissingRequired`] when `required` and no id
    /// - [`PaymentIdError::InvalidId`] when format rules fail
    pub fn validate_payload_entry(
        &self,
        entry: Option<&ExtensionEntry>,
    ) -> Result<Option<String>, PaymentIdError> {
        let owned = entry.map(ExtensionEntry::to_value);
        let id = owned.as_ref().and_then(extract_id).map(str::to_owned);
        match id {
            None if self.config.required => Err(PaymentIdError::MissingRequired),
            None => Ok(None),
            Some(raw) => {
                validate_id_format(&raw)?;
                Ok(Some(raw))
            }
        }
    }
}

impl Default for PaymentIdentifierExtension {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors from payment-identifier validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PaymentIdError {
    /// Server set `required: true` but the client omitted `id`.
    #[error("payment-identifier required but missing")]
    MissingRequired,
    /// Client `id` fails length or charset rules.
    #[error("invalid payment-identifier id: {0}")]
    InvalidId(String),
}

/// Official `id` format: 16–128 chars, alphanumeric / hyphen / underscore.
///
/// # Errors
///
/// [`PaymentIdError::InvalidId`] when length or charset rules fail.
pub fn validate_id_format(id: &str) -> Result<(), PaymentIdError> {
    let len = id.chars().count();
    if !(ID_MIN_LEN..=ID_MAX_LEN).contains(&len) {
        return Err(PaymentIdError::InvalidId(format!(
            "length {len} outside {ID_MIN_LEN}..={ID_MAX_LEN}"
        )));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(PaymentIdError::InvalidId(
            "must be alphanumeric, hyphen, or underscore".into(),
        ));
    }
    Ok(())
}

/// Extracts the identifier from a payload extension value.
///
/// Accepts official `info.id` and bare `id` (structured or raw entry values).
fn extract_id(value: &Value) -> Option<&str> {
    value
        .get("info")
        .and_then(|info| info.get("id"))
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
}

impl Extension for PaymentIdentifierExtension {
    fn id(&self) -> &'static str {
        PAYMENT_IDENTIFIER_KEY
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
        let entry = ctx.payload.extensions.get(PAYMENT_IDENTIFIER_KEY);
        let outcome = self.validate_payload_entry(entry);
        let result = match outcome {
            Err(PaymentIdError::MissingRequired) => Some(ExtensionEntry::info(json!({
                "required": true,
                "status": "missing_required",
            }))),
            Err(PaymentIdError::InvalidId(msg)) => Some(ExtensionEntry::info(json!({
                "required": self.config.required,
                "status": "invalid",
                "message": msg,
            }))),
            Ok(None) => None,
            Ok(Some(id)) => {
                let status = match self.record(&id) {
                    Duplicate::No => "accepted",
                    Duplicate::Yes => "duplicate",
                };
                Some(ExtensionEntry::info(json!({
                    "required": self.config.required,
                    "id": id,
                    "status": status,
                })))
            }
        };
        std::future::ready(result)
    }

    fn on_settle<'a>(
        &'a self,
        ctx: &SettleContext<'_>,
    ) -> impl Future<Output = Option<ExtensionEntry>> + Send + 'a {
        let entry = ctx.payload.extensions.get(PAYMENT_IDENTIFIER_KEY);
        let id = entry
            .map(ExtensionEntry::to_value)
            .as_ref()
            .and_then(extract_id)
            .map(str::to_owned);
        let result = id.map(|id| {
            ExtensionEntry::info(json!({
                "required": self.config.required,
                "id": id,
                "status": "settled",
            }))
        });
        std::future::ready(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertise_matches_official_shape() {
        let ext = PaymentIdentifierExtension::new().with_required(true);
        let entry = ext
            .advertise(&AdvertiseContext { requirement: None })
            .unwrap();
        let value = entry.to_value();
        assert_eq!(value["info"]["required"], true);
        assert!(value.get("schema").is_some());
        assert!(value["info"].get("idempotency").is_none());
        assert!(value["info"].get("maxAgeSeconds").is_none());
    }

    #[test]
    fn record_dedups() {
        let ext = PaymentIdentifierExtension::new();
        assert_eq!(ext.record("pay_0123456789abcdef"), Duplicate::No);
        assert_eq!(ext.record("pay_0123456789abcdef"), Duplicate::Yes);
    }

    #[test]
    fn extract_id_from_info_envelope() {
        let v = json!({"info": {"required": false, "id": "pay_0123456789abcdef"}});
        assert_eq!(extract_id(&v), Some("pay_0123456789abcdef"));
    }

    #[test]
    fn extract_id_from_bare_object() {
        let v = json!({"id": "pay_0123456789abcdef"});
        assert_eq!(extract_id(&v), Some("pay_0123456789abcdef"));
    }

    #[test]
    fn validate_id_format_rules() {
        assert!(validate_id_format("pay_0123456789ab").is_ok());
        assert!(validate_id_format("short").is_err());
        assert!(validate_id_format("has space!!!!!!").is_err());
    }

    #[test]
    fn validate_required_missing() {
        let ext = PaymentIdentifierExtension::new().with_required(true);
        assert!(matches!(
            ext.validate_payload_entry(None),
            Err(PaymentIdError::MissingRequired)
        ));
    }

    #[test]
    fn validate_optional_missing_ok() {
        let ext = PaymentIdentifierExtension::new();
        assert_eq!(ext.validate_payload_entry(None).unwrap(), None);
    }
}
