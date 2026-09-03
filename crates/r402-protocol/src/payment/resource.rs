//! Resource metadata on a 402 challenge.

use compact_str::CompactString;
use serde::{Deserialize, Serialize};

/// Human-readable metadata describing the paid resource.
///
/// Spec §5.1.2: only `url` is required.
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
    /// Human-readable name of the service hosting the resource.
    ///
    /// Printable ASCII, max 32 characters per spec §5.1.2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_name: Option<CompactString>,
    /// Topical tags for the service, used for discovery filtering.
    ///
    /// Max 5 entries; each printable ASCII, max 32 characters, per spec §5.1.2.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<CompactString>,
    /// Absolute `https`/`http` URL to an icon representing the service.
    ///
    /// Max 2048 characters per spec §5.1.2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<CompactString>,
}

impl ResourceInfo {
    /// Constructs a [`ResourceInfo`] carrying just a URL.
    #[must_use]
    pub fn new(url: impl Into<CompactString>) -> Self {
        Self {
            url: url.into(),
            description: None,
            mime_type: None,
            service_name: None,
            tags: Vec::new(),
            icon_url: None,
        }
    }

    /// Sets `description`.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<CompactString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Sets `mimeType`.
    #[must_use]
    pub fn with_mime_type(mut self, mime_type: impl Into<CompactString>) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }

    /// Sets `serviceName`.
    #[must_use]
    pub fn with_service_name(mut self, service_name: impl Into<CompactString>) -> Self {
        self.service_name = Some(service_name.into());
        self
    }

    /// Replaces the `tags` list.
    #[must_use]
    pub fn with_tags(mut self, tags: Vec<CompactString>) -> Self {
        self.tags = tags;
        self
    }

    /// Appends a single tag.
    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<CompactString>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Sets `iconUrl`.
    #[must_use]
    pub fn with_icon_url(mut self, icon_url: impl Into<CompactString>) -> Self {
        self.icon_url = Some(icon_url.into());
        self
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "unit tests panic on assertion failure"
)]
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
            .with_mime_type("application/json")
            .with_service_name("Example Weather")
            .with_tag("weather")
            .with_tag("forecast")
            .with_icon_url("https://example.com/icon.png");
        let encoded = serde_json::to_value(&info).unwrap();
        assert_eq!(encoded["mimeType"], "application/json");
        assert_eq!(encoded["serviceName"], "Example Weather");
        assert_eq!(encoded["tags"], serde_json::json!(["weather", "forecast"]));
        assert_eq!(encoded["iconUrl"], "https://example.com/icon.png");
        let decoded: ResourceInfo = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, info);
    }

    #[test]
    fn discovery_metadata_omitted_by_default() {
        let info = ResourceInfo::new("https://example.com/r");
        let v = serde_json::to_value(&info).unwrap();
        assert!(v.get("serviceName").is_none());
        assert!(v.get("tags").is_none());
        assert!(v.get("iconUrl").is_none());
    }

    #[test]
    fn deserializes_spec_compliant_optional_fields() {
        let json = serde_json::json!({ "url": "https://x.test" });
        let decoded: ResourceInfo = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.url, "https://x.test");
        assert!(decoded.description.is_none());
        assert!(decoded.mime_type.is_none());
    }

    #[test]
    fn rejects_unknown_field() {
        let json = serde_json::json!({ "url": "https://x.test", "unknown": 1 });
        assert!(serde_json::from_value::<ResourceInfo>(json).is_err());
    }
}
