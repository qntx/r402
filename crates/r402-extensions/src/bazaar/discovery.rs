//! Facilitator `GET /discovery/resources` and `GET /discovery/search` client.
//!
//! Official `@x402/extensions/bazaar` `withBazaar` / `facilitatorClient.ts`.

use std::time::Duration;

use compact_str::CompactString;
use r402_facilitator::{FacilitatorClient, FacilitatorClientError};
use reqwest::Client;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

/// Query filters for `GET /discovery/resources`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListDiscoveryResourcesParams {
    /// Query parameter `type` (e.g. `"http"`, `"mcp"`).
    pub resource_type: Option<CompactString>,
    /// Filter by payment recipient address.
    pub pay_to: Option<CompactString>,
    /// Filter by payment scheme (e.g. `"exact"`).
    pub scheme: Option<CompactString>,
    /// Filter by CAIP-2 network (e.g. `"eip155:8453"`).
    pub network: Option<CompactString>,
    /// Filter by extension key present on the discovered resource.
    pub extensions: Option<CompactString>,
    /// Page size.
    pub limit: Option<u64>,
    /// Number of matching resources to skip.
    pub offset: Option<u64>,
}

/// Query filters for `GET /discovery/search`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchDiscoveryResourcesParams {
    /// Natural-language search query.
    pub query: CompactString,
    /// Query parameter `type` (e.g. `"http"`, `"mcp"`).
    pub resource_type: Option<CompactString>,
    /// Filter by payment recipient address.
    pub pay_to: Option<CompactString>,
    /// Filter by payment scheme (e.g. `"exact"`).
    pub scheme: Option<CompactString>,
    /// Filter by CAIP-2 network (e.g. `"eip155:8453"`).
    pub network: Option<CompactString>,
    /// Filter by extension key present on the discovered resource.
    pub extensions: Option<CompactString>,
    /// Advisory maximum number of results.
    pub limit: Option<u64>,
    /// Advisory continuation cursor from a previous response.
    pub cursor: Option<CompactString>,
}

/// A discovered x402 resource from the bazaar catalog.
#[allow(
    clippy::derive_partial_eq_without_eq,
    reason = "accepts and extensions are open JSON (serde_json::Value is not Eq)"
)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryResource {
    /// URL or identifier of the discovered resource.
    pub resource: CompactString,
    /// Protocol type (e.g. `"http"`, `"mcp"`).
    #[serde(rename = "type")]
    pub resource_type: CompactString,
    /// x402 protocol version supported by this resource.
    pub x402_version: u32,
    /// Catalog JSON for each accept. Extra or incomplete keys must not fail the page.
    pub accepts: Vec<Value>,
    /// ISO 8601 timestamp of the last catalog update.
    pub last_updated: CompactString,
    /// Human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<CompactString>,
    /// MIME type of the resource response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<CompactString>,
    /// Human-readable name of the hosting service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_name: Option<CompactString>,
    /// Short topical tags for discovery search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<CompactString>>,
    /// Absolute `http(s)` URL to a service icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<CompactString>,
    /// Extension payloads echoed from discovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Value>,
}

/// Offset pagination on a list response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryPagination {
    /// Maximum number of results returned.
    pub limit: u64,
    /// Number of results skipped.
    pub offset: u64,
    /// Total count of resources matching the query.
    pub total: u64,
}

/// Response from `GET /discovery/resources`.
#[allow(
    clippy::derive_partial_eq_without_eq,
    reason = "items carry open JSON accepts/extensions"
)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryResourcesResponse {
    /// x402 protocol version of this response.
    pub x402_version: u32,
    /// Discovered resources on this page.
    pub items: Vec<DiscoveryResource>,
    /// Offset pagination for the response.
    pub pagination: DiscoveryPagination,
}

/// Cursor pagination on a search response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPagination {
    /// Number of results in this page.
    pub limit: u64,
    /// Continuation cursor for the next page; JSON `null` is [`None`].
    pub cursor: Option<CompactString>,
}

/// Response from `GET /discovery/search`.
///
/// Omitted and JSON-`null` `pagination` both deserialize as [`None`].
#[allow(
    clippy::derive_partial_eq_without_eq,
    reason = "resources carry open JSON accepts/extensions"
)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchDiscoveryResourcesResponse {
    /// x402 protocol version of this response.
    pub x402_version: u32,
    /// Matching discovered resources.
    pub resources: Vec<DiscoveryResource>,
    /// `true` when additional matches were truncated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_results: Option<bool>,
    /// Present when the facilitator paginated the search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<SearchPagination>,
}

/// Failures talking to a facilitator discovery endpoint.
#[derive(Debug, thiserror::Error)]
pub enum BazaarDiscoveryError {
    /// Non-success `GET /discovery/resources`.
    #[error("Facilitator listDiscoveryResources failed ({status}): {body}")]
    ListFailed {
        /// HTTP status code.
        status: u16,
        /// Response body, or the status text if the body could not be read.
        body: String,
    },
    /// Non-success `GET /discovery/search`.
    #[error("Facilitator searchDiscoveryResources failed ({status}): {body}")]
    SearchFailed {
        /// HTTP status code.
        status: u16,
        /// Response body, or the status text if the body could not be read.
        body: String,
    },
    /// 2xx body was not valid discovery JSON.
    #[error("failed to parse discovery response: {0}")]
    Parse(String),
    /// Auth-header callback or URL construction on the inner client failed.
    #[error(transparent)]
    FacilitatorClient(#[from] FacilitatorClientError),
    /// HTTP transport failure.
    #[error("HTTP error: {context}: {source}")]
    Http {
        /// Human-readable context.
        context: &'static str,
        /// Underlying reqwest error.
        #[source]
        source: reqwest::Error,
    },
    /// Discovery path could not be joined onto the facilitator base URL.
    #[error("URL parse error: {context}: {source}")]
    Url {
        /// Human-readable context.
        context: &'static str,
        /// Underlying parse error.
        #[source]
        source: url::ParseError,
    },
}

/// Facilitator client with bazaar `GET /discovery` methods.
#[derive(Clone, Debug)]
pub struct BazaarDiscoveryClient {
    inner: FacilitatorClient,
    http: Client,
}

impl SearchDiscoveryResourcesParams {
    /// Builds search params with a required query string.
    #[must_use]
    pub fn new(query: impl Into<CompactString>) -> Self {
        Self {
            query: query.into(),
            ..Self::default()
        }
    }
}

impl BazaarDiscoveryClient {
    /// Wraps a facilitator client for discovery queries.
    ///
    /// # Errors
    ///
    /// Returns [`BazaarDiscoveryError::Http`] if the HTTP client cannot be built.
    pub fn new(client: FacilitatorClient) -> Result<Self, BazaarDiscoveryError> {
        let http = build_http(client.base_url(), client.timeout())?;
        Ok(Self {
            inner: client,
            http,
        })
    }

    /// Lists cataloged resources. Pass [`ListDiscoveryResourcesParams::default`] for no filters.
    ///
    /// # Errors
    ///
    /// Transport, auth, non-success HTTP, or JSON parse failures.
    pub async fn list_resources(
        &self,
        params: &ListDiscoveryResourcesParams,
    ) -> Result<DiscoveryResourcesResponse, BazaarDiscoveryError> {
        self.send(
            "discovery/resources",
            &params.query_pairs(),
            BazaarDiscoveryError::list_failed,
        )
        .await
    }

    /// Searches cataloged resources. `params.query` is always sent.
    ///
    /// # Errors
    ///
    /// Transport, auth, non-success HTTP, or JSON parse failures.
    pub async fn search(
        &self,
        params: &SearchDiscoveryResourcesParams,
    ) -> Result<SearchDiscoveryResourcesResponse, BazaarDiscoveryError> {
        self.send(
            "discovery/search",
            &params.query_pairs(),
            BazaarDiscoveryError::search_failed,
        )
        .await
    }

    async fn send<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        on_error: fn(u16, String) -> BazaarDiscoveryError,
    ) -> Result<T, BazaarDiscoveryError> {
        let url = discovery_url(self.inner.base_url(), path, query)?;
        let headers = self.headers().await?;
        let mut req = self.http.get(url).headers(headers);
        if let Some(timeout) = self.inner.timeout() {
            req = req.timeout(*timeout);
        }
        let response = req.send().await.map_err(map_http)?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_else(|_| status.to_string());
            return Err(on_error(status.as_u16(), body));
        }
        let bytes = response.bytes().await.map_err(map_http)?;
        serde_json::from_slice(&bytes).map_err(|err| BazaarDiscoveryError::Parse(err.to_string()))
    }

    async fn headers(&self) -> Result<HeaderMap, BazaarDiscoveryError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let auth = self.inner.create_auth_headers("bazaar").await?;
        for (name, value) in &auth {
            headers.insert(name.clone(), value.clone());
        }
        Ok(headers)
    }
}

/// Extends a facilitator client with bazaar discovery queries.
///
/// # Errors
///
/// Returns [`BazaarDiscoveryError::Http`] if the HTTP client cannot be built.
pub fn with_bazaar(
    client: FacilitatorClient,
) -> Result<BazaarDiscoveryClient, BazaarDiscoveryError> {
    BazaarDiscoveryClient::new(client)
}

impl ListDiscoveryResourcesParams {
    fn query_pairs(&self) -> Vec<(&'static str, String)> {
        collect_query([
            ("type", opt_str(self.resource_type.as_deref())),
            ("payTo", opt_str(self.pay_to.as_deref())),
            ("scheme", opt_str(self.scheme.as_deref())),
            ("network", opt_str(self.network.as_deref())),
            ("extensions", opt_str(self.extensions.as_deref())),
            ("limit", self.limit.map(|n| n.to_string())),
            ("offset", self.offset.map(|n| n.to_string())),
        ])
    }
}

impl SearchDiscoveryResourcesParams {
    fn query_pairs(&self) -> Vec<(&'static str, String)> {
        let mut pairs = vec![("query", self.query.to_string())];
        pairs.extend(collect_query([
            ("type", opt_str(self.resource_type.as_deref())),
            ("payTo", opt_str(self.pay_to.as_deref())),
            ("scheme", opt_str(self.scheme.as_deref())),
            ("network", opt_str(self.network.as_deref())),
            ("extensions", opt_str(self.extensions.as_deref())),
            ("limit", self.limit.map(|n| n.to_string())),
            ("cursor", opt_str(self.cursor.as_deref())),
        ]));
        pairs
    }
}

impl BazaarDiscoveryError {
    const fn list_failed(status: u16, body: String) -> Self {
        Self::ListFailed { status, body }
    }

    const fn search_failed(status: u16, body: String) -> Self {
        Self::SearchFailed { status, body }
    }
}

fn opt_str(value: Option<&str>) -> Option<String> {
    value.map(ToOwned::to_owned)
}

fn collect_query<const N: usize>(
    extra: [(&'static str, Option<String>); N],
) -> Vec<(&'static str, String)> {
    extra
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key, value)))
        .collect()
}

fn discovery_url(
    base: &Url,
    path: &str,
    query: &[(&str, String)],
) -> Result<Url, BazaarDiscoveryError> {
    let mut url = base
        .join(path)
        .map_err(|source| BazaarDiscoveryError::Url {
            context: "Failed to construct bazaar discovery URL",
            source,
        })?;
    if query.is_empty() {
        return Ok(url);
    }
    {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key, value);
        }
    }
    Ok(url)
}

fn build_http(base: &Url, timeout: Option<&Duration>) -> Result<Client, BazaarDiscoveryError> {
    let mut builder = Client::builder();
    if let Some(timeout) = timeout {
        builder = builder.timeout(*timeout);
    }
    if is_loopback(base) {
        // HTTP_PROXY must not capture loopback mock servers or local facilitators.
        builder = builder.no_proxy();
    }
    builder
        .build()
        .map_err(|source| BazaarDiscoveryError::Http {
            context: "failed to build bazaar discovery HTTP client",
            source,
        })
}

const fn map_http(source: reqwest::Error) -> BazaarDiscoveryError {
    BazaarDiscoveryError::Http {
        context: "bazaar discovery request failed",
        source,
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(addr)) => addr.is_loopback(),
        Some(url::Host::Ipv6(addr)) => addr.is_loopback(),
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "unit tests panic on assertion failure")]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://x402.org/facilitator/").unwrap()
    }

    #[test]
    fn list_url_omits_query_when_empty() {
        let url = discovery_url(&base(), "discovery/resources", &[]).unwrap();
        assert_eq!(
            url.as_str(),
            "https://x402.org/facilitator/discovery/resources"
        );
        assert_eq!(url.query(), None);
    }

    #[test]
    fn list_url_encodes_filters() {
        let params = ListDiscoveryResourcesParams {
            resource_type: Some("http".into()),
            pay_to: Some("0x1234567890123456789012345678901234567890".into()),
            scheme: Some("exact".into()),
            network: Some("eip155:8453".into()),
            extensions: Some("bazaar".into()),
            limit: Some(10),
            offset: Some(5),
        };
        let url = discovery_url(&base(), "discovery/resources", &params.query_pairs()).unwrap();
        let query = url.query().unwrap();
        assert!(query.contains("type=http"));
        assert!(query.contains("payTo=0x1234567890123456789012345678901234567890"));
        assert!(query.contains("scheme=exact"));
        assert!(query.contains("network=eip155%3A8453"));
        assert!(query.contains("extensions=bazaar"));
        assert!(query.contains("limit=10"));
        assert!(query.contains("offset=5"));
    }

    #[test]
    fn search_url_always_includes_query() {
        let params = SearchDiscoveryResourcesParams::new("weather APIs");
        let url = discovery_url(&base(), "discovery/search", &params.query_pairs()).unwrap();
        assert!(url.path().ends_with("/discovery/search"));
        assert_eq!(url.query().unwrap(), "query=weather+APIs");
    }

    #[test]
    fn accepts_keeps_incomplete_and_unknown_keys() {
        let resource: DiscoveryResource = serde_json::from_value(serde_json::json!({
            "resource": "https://api.example.com/weather",
            "type": "http",
            "x402Version": 2,
            "accepts": [
                {"scheme": "exact", "network": "eip155:8453"},
                {
                    "scheme": "exact",
                    "network": "eip155:8453",
                    "asset": "0x0",
                    "amount": "1",
                    "payTo": "0x1",
                    "maxTimeoutSeconds": 60,
                    "description": "v1 leftover",
                    "resource": "https://api.example.com/weather",
                    "maxAmountRequired": "1"
                }
            ],
            "lastUpdated": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        assert_eq!(resource.accepts.len(), 2);
        assert_eq!(resource.accepts[0]["scheme"], "exact");
        assert!(resource.accepts[0].get("asset").is_none());
        assert_eq!(resource.accepts[1]["description"], "v1 leftover");
        assert_eq!(resource.accepts[1]["maxAmountRequired"], "1");
    }
}
