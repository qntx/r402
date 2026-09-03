//! x402 protocol extension envelope.

use std::collections::HashMap;

use compact_str::CompactString;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Field names from an `EXTENSION-RESPONSES` value that may be logged.
pub const EXTENSION_RESPONSE_LOG_FIELDS: &[&str] = &["status", "rejectedReason", "reason", "code"];

/// Canonical x402 extension envelope: extension id → [`ExtensionEntry`].
///
/// # Examples
///
/// ```
/// use r402_protocol::payment::{ExtensionEntry, Extensions};
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
    /// Empty extension map.
    #[must_use]
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    /// Whether no extensions are present.
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

    /// Removes an extension by stable ID.
    #[must_use]
    pub fn remove(&mut self, id: &str) -> Option<ExtensionEntry> {
        self.0.remove(id)
    }

    /// Iterates over `(id, entry)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&CompactString, &ExtensionEntry)> {
        self.0.iter()
    }

    /// Inserts every entry from `other`, overwriting on id collision.
    pub fn extend(&mut self, other: Self) {
        self.0.extend(other.0);
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
/// Structured form is `{info, schema?, …}`. Sibling keys such as SIWX
/// `supportedChains` are preserved on deserialize so buyers can read them.
/// Raw form is forwarded verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExtensionEntry {
    /// Canonical `{info, schema?}` envelope plus any sibling fields.
    Structured {
        /// Extension-specific payload.
        info: Value,
        /// Optional JSON Schema for client-submitted fields.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<Value>,
        /// Sibling keys (`supportedChains`, …). Empty when absent.
        #[serde(default, flatten)]
        extra: serde_json::Map<String, Value>,
    },
    /// Raw JSON payload, forwarded as-is.
    Raw(Value),
}

impl ExtensionEntry {
    /// Structured entry with only `info`.
    #[must_use]
    pub fn info(info: Value) -> Self {
        Self::Structured {
            info,
            schema: None,
            extra: serde_json::Map::new(),
        }
    }

    /// Structured entry with `info` and `schema`.
    #[must_use]
    pub fn with_schema(info: Value, schema: Value) -> Self {
        Self::Structured {
            info,
            schema: Some(schema),
            extra: serde_json::Map::new(),
        }
    }

    /// Raw entry wrapping the supplied JSON value.
    #[must_use]
    pub const fn raw(value: Value) -> Self {
        Self::Raw(value)
    }

    /// `info` payload if this entry is structured.
    #[must_use]
    pub const fn as_info(&self) -> Option<&Value> {
        match self {
            Self::Structured { info, .. } => Some(info),
            Self::Raw(_) => None,
        }
    }

    /// Schema if present.
    #[must_use]
    pub const fn as_schema(&self) -> Option<&Value> {
        match self {
            Self::Structured { schema, .. } => schema.as_ref(),
            Self::Raw(_) => None,
        }
    }

    /// Raw JSON value regardless of shape.
    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Structured {
                info,
                schema,
                extra,
            } => {
                let mut obj = extra.clone();
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
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "unit tests panic on assertion failure"
)]
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

    #[test]
    fn log_field_allowlist() {
        assert_eq!(
            EXTENSION_RESPONSE_LOG_FIELDS,
            ["status", "rejectedReason", "reason", "code"]
        );
    }

    #[test]
    fn structured_preserves_sibling_fields() {
        let encoded = json!({
            "sign-in-with-x": {
                "info": {"domain": "api.example.com"},
                "supportedChains": [{"chainId": "eip155:8453", "type": "eip191"}]
            }
        });
        let decoded: Extensions = serde_json::from_value(encoded).unwrap();
        let value = decoded.get("sign-in-with-x").unwrap().to_value();
        assert_eq!(value["info"]["domain"], "api.example.com");
        assert_eq!(value["supportedChains"][0]["chainId"], "eip155:8453");
        assert_eq!(value["supportedChains"][0]["type"], "eip191");
    }
}
