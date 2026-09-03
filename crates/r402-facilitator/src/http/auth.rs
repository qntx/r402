//! Path-keyed authentication headers for a remote facilitator.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::{HeaderMap, HeaderName, HeaderValue};

use super::client::FacilitatorClientError;

pub(super) type BoxedAuthHeadersFuture =
    Pin<Box<dyn Future<Output = Result<serde_json::Value, FacilitatorClientError>> + Send>>;
pub(super) type BoxedCreateAuthHeaders = dyn Fn() -> BoxedAuthHeadersFuture + Send + Sync;

/// Path-keyed authentication headers for a remote facilitator.
///
/// Matches the official `createAuthHeaders` result shape. [`Self::bazaar`] is
/// part of that shape; r402 has no bazaar URL and does not send it.
#[derive(Clone, Debug, Default)]
pub struct FacilitatorAuthHeaders {
    /// Headers for `POST /verify`.
    pub verify: HeaderMap,
    /// Headers for `POST /settle`.
    pub settle: HeaderMap,
    /// Headers for `GET /supported`.
    pub supported: HeaderMap,
    /// Official field; unused (no bazaar URL).
    pub bazaar: HeaderMap,
}

impl FacilitatorAuthHeaders {
    pub(super) fn from_json(value: &serde_json::Value) -> Result<Self, FacilitatorClientError> {
        if looks_flat(value) {
            return Err(FacilitatorClientError::FlatAuthHeaders);
        }
        Ok(Self {
            verify: header_map_at(value, "verify")?,
            settle: header_map_at(value, "settle")?,
            supported: header_map_at(value, "supported")?,
            bazaar: header_map_at(value, "bazaar")?,
        })
    }

    pub(super) fn for_path(&self, path: &str) -> HeaderMap {
        match path {
            "verify" => self.verify.clone(),
            "settle" => self.settle.clone(),
            "supported" => self.supported.clone(),
            "bazaar" => self.bazaar.clone(),
            _ => HeaderMap::new(),
        }
    }
}

#[derive(Clone)]
pub(super) struct CreateAuthHeadersCallback(pub Arc<BoxedCreateAuthHeaders>);

impl std::fmt::Debug for CreateAuthHeadersCallback {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CreateAuthHeaders")
    }
}

fn is_header_object(value: &serde_json::Value) -> bool {
    value.is_object()
}

fn looks_flat(auth_headers: &serde_json::Value) -> bool {
    let Some(obj) = auth_headers.as_object() else {
        return true;
    };
    let has_path_key = ["verify", "settle", "supported", "bazaar"]
        .into_iter()
        .any(|key| obj.get(key).is_some_and(is_header_object));
    let some_not_header = obj.values().any(|value| !is_header_object(value));
    !has_path_key && some_not_header
}

fn header_map_at(
    value: &serde_json::Value,
    path: &'static str,
) -> Result<HeaderMap, FacilitatorClientError> {
    match value.get(path) {
        Some(headers) if is_header_object(headers) => json_object_to_header_map(headers, path),
        _ => Ok(HeaderMap::new()),
    }
}

fn json_object_to_header_map(
    value: &serde_json::Value,
    path: &'static str,
) -> Result<HeaderMap, FacilitatorClientError> {
    let Some(obj) = value.as_object() else {
        return Ok(HeaderMap::new());
    };
    let mut headers = HeaderMap::new();
    for (key, val) in obj {
        let Some(val) = val.as_str() else {
            return Err(FacilitatorClientError::InvalidAuthHeader {
                path,
                name: key.clone(),
            });
        };
        let Ok(name) = HeaderName::try_from(key.as_str()) else {
            return Err(FacilitatorClientError::InvalidAuthHeader {
                path,
                name: key.clone(),
            });
        };
        let Ok(header_value) = HeaderValue::try_from(val) else {
            return Err(FacilitatorClientError::InvalidAuthHeader {
                path,
                name: key.clone(),
            });
        };
        headers.insert(name, header_value);
    }
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::looks_flat;

    #[test]
    fn empty_object_is_not_flat() {
        assert!(!looks_flat(&json!({})));
    }

    #[test]
    fn authorization_string_is_flat() {
        assert!(looks_flat(&json!({ "Authorization": "Bearer token" })));
    }

    #[test]
    fn non_object_values_are_flat() {
        assert!(looks_flat(&json!({ "Authorization": 123 })));
    }

    #[test]
    fn nested_non_path_object_is_not_flat() {
        assert!(!looks_flat(&json!({ "X-Api-Key": { "nested": "1" } })));
    }

    #[test]
    fn bazaar_path_key_is_not_flat() {
        assert!(!looks_flat(
            &json!({ "bazaar": { "Authorization": "Bearer bazaar" } })
        ));
    }

    #[test]
    fn non_object_root_is_flat() {
        assert!(looks_flat(&json!(["verify"])));
    }
}
