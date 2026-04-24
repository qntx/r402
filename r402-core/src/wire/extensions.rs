//! x402 protocol extension envelope.
//!
//! Per the x402 v2 spec §Extensions, every wire message carrying extensions
//! uses a JSON object keyed by the stable extension ID. Each value carries an
//! extension-specific payload; the canonical shape is `{ info, schema }` but
//! extensions may choose to supply any JSON payload. This module models both
//! via [`ExtensionEntry`], which transparently serializes either form.

use std::collections::HashMap;

use compact_str::CompactString;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Canonical x402 extension envelope.
///
/// Maps each extension's stable identifier (e.g. `"bazaar"`,
/// `"payment-identifier"`) to its [`ExtensionEntry`]. The type is
/// `#[serde(transparent)]` so on the wire it appears as a plain JSON object.
///
/// # Examples
///
/// ```
/// use r402_core::wire::{Extensions, ExtensionEntry};
/// use serde_json::json;
///
/// let mut ext = Extensions::new();
/// ext.insert("bazaar", ExtensionEntry::info(json!({"registered": true})));
/// let rendered = serde_json::to_value(&ext).unwrap();
/// assert_eq!(rendered["bazaar"]["info"]["registered"], true);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Extensions(HashMap<CompactString, ExtensionEntry>);

impl Extensions {
    /// Creates an empty extension map.
    #[must_use]
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    /// Returns `true` when no extensions are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of registered extensions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Looks up an extension by stable ID.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&ExtensionEntry> {
        self.0.get(id)
    }

    /// Inserts or replaces an extension payload for the given ID.
    pub fn insert(&mut self, id: impl Into<CompactString>, entry: ExtensionEntry) {
        let _ = self.0.insert(id.into(), entry);
    }

    /// Removes an extension by stable ID, returning the previous value if any.
    #[must_use]
    pub fn remove(&mut self, id: &str) -> Option<ExtensionEntry> {
        self.0.remove(id)
    }

    /// Iterates over `(id, entry)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&CompactString, &ExtensionEntry)> {
        self.0.iter()
    }
}

impl<K, V> FromIterator<(K, V)> for Extensions
where
    K: Into<CompactString>,
    V: Into<ExtensionEntry>,
{
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        Self(
            iter.into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        )
    }
}

/// Payload attached to a single extension key.
///
/// Two shapes are permitted by the spec:
///
/// 1. [`ExtensionEntry::Structured`] — the canonical `{info, schema}` envelope
///    recommended for new extensions. `schema` is optional and, when present,
///    should describe the expected shape of client-submitted data.
/// 2. [`ExtensionEntry::Raw`] — an opaque JSON value, forwarded verbatim.
///    Used when an extension predates the canonical envelope or needs a
///    non-object top-level value.
///
/// Serialization preserves the original form: structured values emit
/// `{info, schema?}`; raw values emit their stored JSON directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExtensionEntry {
    /// Canonical `{info, schema?}` envelope.
    Structured {
        /// Extension-specific payload.
        info: Value,
        /// Optional JSON Schema for client-submitted fields.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<Value>,
    },
    /// Raw JSON payload, forwarded as-is.
    Raw(Value),
}

impl ExtensionEntry {
    /// Constructs a structured entry with only `info`.
    #[must_use]
    pub const fn info(info: Value) -> Self {
        Self::Structured { info, schema: None }
    }

    /// Constructs a structured entry with `info` and `schema`.
    #[must_use]
    pub const fn with_schema(info: Value, schema: Value) -> Self {
        Self::Structured {
            info,
            schema: Some(schema),
        }
    }

    /// Constructs a raw entry wrapping the supplied JSON value.
    #[must_use]
    pub const fn raw(value: Value) -> Self {
        Self::Raw(value)
    }

    /// Returns the `info` payload if this entry is structured.
    #[must_use]
    pub const fn as_info(&self) -> Option<&Value> {
        match self {
            Self::Structured { info, .. } => Some(info),
            Self::Raw(_) => None,
        }
    }

    /// Returns the schema if present.
    #[must_use]
    pub const fn as_schema(&self) -> Option<&Value> {
        match self {
            Self::Structured { schema, .. } => schema.as_ref(),
            Self::Raw(_) => None,
        }
    }

    /// Returns the raw JSON value regardless of shape.
    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Structured { info, schema } => {
                let mut obj = serde_json::Map::new();
                let _ = obj.insert("info".to_owned(), info.clone());
                if let Some(schema) = schema {
                    let _ = obj.insert("schema".to_owned(), schema.clone());
                }
                Value::Object(obj)
            }
            Self::Raw(value) => value.clone(),
        }
    }
}

impl From<Value> for ExtensionEntry {
    fn from(value: Value) -> Self {
        Self::Raw(value)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn extensions_empty_by_default() {
        let ext = Extensions::new();
        assert!(ext.is_empty());
        assert_eq!(serde_json::to_value(&ext).unwrap(), json!({}));
    }

    #[test]
    fn structured_entry_roundtrip() {
        let mut ext = Extensions::new();
        ext.insert(
            "bazaar",
            ExtensionEntry::with_schema(json!({"registered": true}), json!({"type": "object"})),
        );
        let encoded = serde_json::to_value(&ext).unwrap();
        assert_eq!(encoded["bazaar"]["info"]["registered"], true);
        assert_eq!(encoded["bazaar"]["schema"]["type"], "object");
        let decoded: Extensions = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, ext);
    }

    #[test]
    fn raw_entry_roundtrip() {
        let mut ext = Extensions::new();
        ext.insert("custom", ExtensionEntry::raw(json!([1, 2, 3])));
        let encoded = serde_json::to_value(&ext).unwrap();
        assert_eq!(encoded["custom"], json!([1, 2, 3]));
        let decoded: Extensions = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, ext);
    }
}
