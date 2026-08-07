//! `payment-identifier` extension — client-supplied idempotency keys.
//!
//! Per the x402 v2 spec, clients may attach a unique identifier to each
//! payment so that the seller + facilitator can deduplicate settlement on
//! retry. This extension implements the server-side policing.
//!
//! # Wire format
//!
//! ```json
//! {
//!   "extensions": {
//!     "payment-identifier": {
//!       "info": { "maxAgeSeconds": 600, "idempotency": true },
//!       "schema": {
//!         "type": "object",
//!         "properties": { "id": { "type": "string", "maxLength": 128 } },
//!         "required": ["id"]
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! Clients sending a payment with this extension MUST place the id in
//! `payment_payload.extensions["payment-identifier"]["id"]` (or `info.id`,
//! both shapes are accepted).

#![cfg(feature = "ext-payment-id")]
#![cfg_attr(docsrs, doc(cfg(feature = "ext-payment-id")))]

use std::future::Future;
use std::time::Duration;

use serde_json::{Value, json};

use super::{AdvertiseContext, Extension, SettleContext, VerifyContext};
use crate::cache::{Duplicate, TtlSet};
use crate::wire::ExtensionEntry;

/// Default TTL for remembered payment identifiers.
pub const DEFAULT_MAX_AGE: Duration = Duration::from_mins(10);
/// Default upper bound on tracked identifiers.
pub const DEFAULT_CAPACITY: u64 = 10_000;

/// Configuration for [`PaymentIdentifierExtension`].
#[derive(Debug, Clone, Copy)]
pub struct PaymentIdentifierConfig {
    /// How long to remember each identifier.
    pub max_age: Duration,
    /// Maximum number of distinct identifiers tracked at once.
    pub capacity: u64,
}

impl Default for PaymentIdentifierConfig {
    fn default() -> Self {
        Self {
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
    /// Constructs an extension with default configuration.
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

    /// Records the identifier and returns whether it was newly seen.
    pub fn record(&self, id: &str) -> Duplicate {
        self.cache.reserve(id)
    }

    /// Returns the backing TTL set (primarily for inspection in tests).
    #[must_use]
    pub const fn cache(&self) -> &TtlSet {
        &self.cache
    }
}

impl Default for PaymentIdentifierExtension {
    fn default() -> Self {
        Self::new()
    }
}

/// Extracts the identifier from a payload extension entry, accepting either
/// `{"id": "..."}` or `{"info": {"id": "..."}}`.
fn extract_id(value: &Value) -> Option<&str> {
    value
        .get("id")
        .or_else(|| value.get("info").and_then(|info| info.get("id")))
        .and_then(Value::as_str)
}

impl Extension for PaymentIdentifierExtension {
    fn id(&self) -> &'static str {
        "payment-identifier"
    }

    fn advertise(&self, _ctx: &AdvertiseContext<'_>) -> Option<ExtensionEntry> {
        Some(ExtensionEntry::with_schema(
            json!({
                "idempotency": true,
                "maxAgeSeconds": self.config.max_age.as_secs(),
            }),
            json!({
                "type": "object",
                "properties": { "id": { "type": "string", "maxLength": 128 } },
                "required": ["id"],
            }),
        ))
    }

    fn on_verify<'a>(
        &'a self,
        ctx: &VerifyContext<'_>,
    ) -> impl Future<Output = Option<ExtensionEntry>> + Send + 'a {
        let entry = ctx.payload.extensions.get("payment-identifier");
        let value = entry.map(ExtensionEntry::to_value);
        let id = value.as_ref().and_then(|v| extract_id(v));
        let Some(id) = id else {
            return std::future::ready(None);
        };
        let status = match self.record(id) {
            Duplicate::No => "accepted",
            Duplicate::Yes => "duplicate",
        };
        std::future::ready(Some(ExtensionEntry::info(json!({
            "id": id,
            "status": status,
        }))))
    }

    fn on_settle<'a>(
        &'a self,
        ctx: &SettleContext<'_>,
    ) -> impl Future<Output = Option<ExtensionEntry>> + Send + 'a {
        let entry = ctx.payload.extensions.get("payment-identifier");
        let value = entry.map(ExtensionEntry::to_value);
        let id = value.as_ref().and_then(|v| extract_id(v));
        let Some(id) = id else {
            return std::future::ready(None);
        };
        std::future::ready(Some(ExtensionEntry::info(json!({
            "id": id,
            "status": "settled",
        }))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_dedups() {
        let ext = PaymentIdentifierExtension::new();
        assert_eq!(ext.record("order-1"), Duplicate::No);
        assert_eq!(ext.record("order-1"), Duplicate::Yes);
        assert_eq!(ext.record("order-2"), Duplicate::No);
    }

    #[test]
    fn extract_id_from_bare_object() {
        let v = json!({"id": "abc"});
        assert_eq!(extract_id(&v), Some("abc"));
    }

    #[test]
    fn extract_id_from_info_envelope() {
        let v = json!({"info": {"id": "xyz"}});
        assert_eq!(extract_id(&v), Some("xyz"));
    }

    #[test]
    fn extract_id_missing_returns_none() {
        assert!(extract_id(&json!({})).is_none());
        assert!(extract_id(&json!({"info": {}})).is_none());
    }
}
