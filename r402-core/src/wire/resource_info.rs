//! Resource metadata attached to `PaymentRequired` / `PaymentPayload`.

use compact_str::CompactString;
use serde::{Deserialize, Serialize};

/// Human-readable metadata describing the paid resource.
///
/// Per the x402 v2 spec, only `url` is required. `description` and
/// `mimeType` are optional because many resources (e.g. raw API endpoints)
/// have no meaningful MIME type or prose description.
///
/// The field names use `camelCase` to align with the wire format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct ResourceInfo {
    /// Canonical URL of the resource.
    pub url: CompactString,
    /// Optional human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<CompactString>,
    /// Optional MIME type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<CompactString>,
}

impl ResourceInfo {
    /// Constructs a [`ResourceInfo`] carrying just a URL.
    #[must_use]
    pub fn new(url: impl Into<CompactString>) -> Self {
        Self {
            url: url.into(),
            description: None,
            mime_type: None,
        }
    }

    /// Builder: sets `description`.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<CompactString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Builder: sets `mimeType`.
    #[must_use]
    pub fn with_mime_type(mut self, mime_type: impl Into<CompactString>) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_resource_omits_optional_fields() {
        let info = ResourceInfo::new("https://example.com/paid");
        let v = serde_json::to_value(&info).unwrap();
        assert_eq!(v["url"], "https://example.com/paid");
        assert!(v.get("description").is_none());
        assert!(v.get("mimeType").is_none());
    }

    #[test]
    fn full_resource_roundtrips() {
        let info = ResourceInfo::new("https://example.com/r")
            .with_description("doc")
            .with_mime_type("application/json");
        let encoded = serde_json::to_value(&info).unwrap();
        assert_eq!(encoded["mimeType"], "application/json");
        let decoded: ResourceInfo = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, info);
    }

    #[test]
    fn deserializes_spec_compliant_optional_fields() {
        let json = serde_json::json!({ "url": "https://x.test" });
        let decoded: ResourceInfo = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.url, "https://x.test");
        assert!(decoded.description.is_none());
        assert!(decoded.mime_type.is_none());
    }

    /// F-001 regression: unknown top-level field is rejected.
    #[test]
    fn rejects_unknown_field() {
        let json = serde_json::json!({ "url": "https://x.test", "unknown": 1 });
        assert!(serde_json::from_value::<ResourceInfo>(json).is_err());
    }
}
