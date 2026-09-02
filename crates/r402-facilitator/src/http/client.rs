//! HTTP client for a remote x402 facilitator.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use http::{HeaderMap, StatusCode};
use r402_protocol::error::{FacilitatorError, FacilitatorTransportKind};
use r402_protocol::payment::{
    Base64Bytes, Extensions, SettleRequest, SettleResponse, SupportedResponse, VerifyRequest,
    VerifyResponse,
};
use reqwest::Client;
use serde::de::DeserializeOwned;
use tokio::sync::RwLock;
use url::Url;

use super::auth::{CreateAuthHeadersCallback, FacilitatorAuthHeaders};
use super::retry::{GET_SUPPORTED_RETRIES, supported_retry_delay};
use crate::Facilitator;

/// TTL cache for [`SupportedResponse`].
#[derive(Clone, Debug)]
struct SupportedCacheState {
    response: SupportedResponse,
    expires_at: std::time::Instant,
}

/// An encapsulated TTL cache for the `/supported` endpoint response.
///
/// Clones share the same cache state via `Arc`.
#[derive(Debug, Clone)]
pub struct SupportedCache {
    ttl: Duration,
    state: Arc<RwLock<Option<SupportedCacheState>>>,
}

impl SupportedCache {
    /// Creates a new cache with the given TTL.
    #[must_use]
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            state: Arc::new(RwLock::new(None)),
        }
    }

    /// Returns the cached response if valid, None otherwise.
    #[allow(
        clippy::significant_drop_tightening,
        reason = "read guard scope matches data access"
    )]
    pub async fn get(&self) -> Option<SupportedResponse> {
        let guard = self.state.read().await;
        let cache = guard.as_ref()?;
        if std::time::Instant::now() < cache.expires_at {
            Some(cache.response.clone())
        } else {
            None
        }
    }

    /// Stores a response in the cache with the configured TTL.
    pub async fn set(&self, response: SupportedResponse) {
        let mut guard = self.state.write().await;
        *guard = Some(SupportedCacheState {
            response,
            expires_at: std::time::Instant::now() + self.ttl,
        });
    }

    /// Clears the cache.
    pub async fn clear(&self) {
        let mut guard = self.state.write().await;
        *guard = None;
    }
}

/// A client for communicating with a remote x402 facilitator.
#[derive(Clone, Debug)]
pub struct FacilitatorClient {
    base_url: Url,
    verify_url: Url,
    settle_url: Url,
    supported_url: Url,
    client: Client,
    timeout: Option<Duration>,
    supported_cache: Option<SupportedCache>,
    create_auth_headers: Option<CreateAuthHeadersCallback>,
}

/// Errors that can occur while constructing a client or resolving auth headers.
#[derive(Debug, thiserror::Error)]
pub enum FacilitatorClientError {
    /// URL parse error.
    #[error("URL parse error: {context}: {source}")]
    UrlParse {
        /// Human-readable context.
        context: &'static str,
        /// The underlying parse error.
        #[source]
        source: url::ParseError,
    },
    /// HTTP transport error while building the reqwest client.
    #[error("HTTP error: {context}: {source}")]
    Http {
        /// Human-readable context.
        context: &'static str,
        /// The underlying reqwest error.
        #[source]
        source: reqwest::Error,
    },
    /// Auth callback returned a flat headers object instead of path keys.
    #[error(
        "createAuthHeaders must return an object keyed by facilitator path, e.g. \
         {{ verify: {{ Authorization: \"...\" }}, settle: {{ ... }}, supported: {{ ... }} }}, \
         but received a flat headers object. See \
         https://github.com/x402-foundation/x402/issues/2762"
    )]
    FlatAuthHeaders,
    /// Auth callback failed before producing headers.
    #[error("createAuthHeaders failed: {0}")]
    Auth(String),
    /// A path-keyed auth object contained a non-string or illegal header.
    #[error("createAuthHeaders {path} header {name} is not a valid string header value")]
    InvalidAuthHeader {
        /// Facilitator path (`verify`, `settle`, `supported`, or `bazaar`).
        path: &'static str,
        /// Header name as given in the JSON object.
        name: String,
    },
}

struct RawHttpResponse {
    status: StatusCode,
    body: Vec<u8>,
    headers: HeaderMap,
    retry_after: Option<String>,
}

impl FacilitatorClient {
    /// Suggested TTL when opting into `/supported` caching.
    pub const DEFAULT_SUPPORTED_CACHE_TTL: Duration = Duration::from_mins(10);

    /// Default per-request timeout. Aligned with the Go/TS clients (30 s).
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

    /// Returns the base URL used by this client.
    #[must_use]
    pub const fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Returns the computed `./verify` URL relative to [`Self::base_url`].
    #[must_use]
    pub const fn verify_url(&self) -> &Url {
        &self.verify_url
    }

    /// Returns the computed `./settle` URL relative to [`Self::base_url`].
    #[must_use]
    pub const fn settle_url(&self) -> &Url {
        &self.settle_url
    }

    /// Returns the computed `./supported` URL relative to [`Self::base_url`].
    #[must_use]
    pub const fn supported_url(&self) -> &Url {
        &self.supported_url
    }

    /// Returns the configured timeout, if any.
    #[must_use]
    pub const fn timeout(&self) -> Option<&Duration> {
        self.timeout.as_ref()
    }

    /// Returns the supported cache when one is configured.
    #[must_use]
    pub const fn supported_cache(&self) -> Option<&SupportedCache> {
        self.supported_cache.as_ref()
    }

    /// Constructs a new [`FacilitatorClient`] from a base URL.
    ///
    /// Sets up `./verify`, `./settle`, and `./supported` relative to the base.
    /// `/supported` is uncached until [`Self::with_supported_cache_ttl`] is called.
    ///
    /// # Errors
    ///
    /// Returns [`FacilitatorClientError`] if URL construction fails.
    pub fn try_new(base_url: Url) -> Result<Self, FacilitatorClientError> {
        let mut builder = Client::builder().timeout(Self::DEFAULT_TIMEOUT);
        if is_loopback(&base_url) {
            // HTTP_PROXY must not capture loopback mock servers or local facilitators.
            builder = builder.no_proxy();
        }
        let client = builder.build().map_err(|e| FacilitatorClientError::Http {
            context: "failed to build reqwest client with default timeout",
            source: e,
        })?;
        let verify_url = join_endpoint(&base_url, "./verify")?;
        let settle_url = join_endpoint(&base_url, "./settle")?;
        let supported_url = join_endpoint(&base_url, "./supported")?;
        Ok(Self {
            client,
            base_url,
            verify_url,
            settle_url,
            supported_url,
            timeout: Some(Self::DEFAULT_TIMEOUT),
            supported_cache: None,
            create_auth_headers: None,
        })
    }

    /// Sets the callback that produces path-keyed facilitator auth headers.
    ///
    /// The JSON object must be keyed by `verify`, `settle`, `supported`, and
    /// optionally `bazaar`. A flat headers object is rejected when headers
    /// are resolved.
    #[must_use]
    pub fn with_auth<F, Fut>(mut self, create: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<serde_json::Value, FacilitatorClientError>> + Send + 'static,
    {
        self.create_auth_headers = Some(CreateAuthHeadersCallback(Arc::new(move || {
            Box::pin(create())
        })));
        self
    }

    /// Sets a timeout for all future requests.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Enables TTL caching of the `/supported` response.
    #[must_use]
    pub fn with_supported_cache_ttl(mut self, ttl: Duration) -> Self {
        self.supported_cache = Some(SupportedCache::new(ttl));
        self
    }

    /// Disables caching for the `/supported` endpoint.
    #[must_use]
    pub fn without_supported_cache(mut self) -> Self {
        self.supported_cache = None;
        self
    }

    /// Resolves authentication headers for one facilitator path.
    ///
    /// # Errors
    ///
    /// Returns [`FacilitatorClientError::FlatAuthHeaders`] when the callback
    /// result has no object-valued `verify`/`settle`/`supported`/`bazaar` key
    /// and some value is not a header object. Returns
    /// [`FacilitatorClientError::InvalidAuthHeader`] when a path object has a
    /// non-string or illegal header. Propagates callback `Err` values.
    pub async fn create_auth_headers(
        &self,
        path: &str,
    ) -> Result<HeaderMap, FacilitatorClientError> {
        let Some(callback) = &self.create_auth_headers else {
            return Ok(HeaderMap::new());
        };
        let value = (callback.0)().await?;
        let parsed = FacilitatorAuthHeaders::from_json(&value)?;
        Ok(parsed.for_path(path))
    }

    /// Sends a `POST /verify` request to the facilitator.
    ///
    /// Non-2xx bodies that deserialize as [`VerifyResponse`] are returned as
    /// `Ok` (402 at the resource server). Timeout, non-2xx without that JSON,
    /// and malformed 2xx bodies are [`FacilitatorError::Transport`] (502).
    ///
    /// # Errors
    ///
    /// Returns [`FacilitatorError`] for transport, auth, or parse failures.
    pub async fn verify(
        &self,
        request: &VerifyRequest,
    ) -> Result<VerifyResponse, FacilitatorError> {
        let headers = self.auth_headers("verify").await?;
        let raw = self
            .send(
                self.client.post(self.verify_url.clone()).json(request),
                &headers,
            )
            .await?;
        parse_operation_body(&raw, attach_verify)
    }

    /// Sends a `POST /settle` request to the facilitator.
    ///
    /// Non-2xx bodies that deserialize as [`SettleResponse`] are returned as
    /// `Ok`. Transport failures follow the same three-state mapping as
    /// [`Self::verify`].
    ///
    /// # Errors
    ///
    /// Returns [`FacilitatorError`] for transport, auth, or parse failures.
    pub async fn settle(
        &self,
        request: &SettleRequest,
    ) -> Result<SettleResponse, FacilitatorError> {
        let headers = self.auth_headers("settle").await?;
        let raw = self
            .send(
                self.client.post(self.settle_url.clone()).json(request),
                &headers,
            )
            .await?;
        parse_operation_body(&raw, attach_settle)
    }

    /// Sends a `GET /supported` request to the facilitator.
    /// Results are cached only when a TTL was configured.
    ///
    /// # Errors
    ///
    /// Returns [`FacilitatorError`] if the HTTP request fails.
    pub async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
        if let Some(cache) = &self.supported_cache
            && let Some(response) = cache.get().await
        {
            return Ok(response);
        }

        #[cfg(feature = "telemetry")]
        if self.supported_cache.is_some() {
            tracing::info!("x402.facilitator_client.supported_cache_miss");
        }

        let response = self.supported_inner().await?;
        if let Some(cache) = &self.supported_cache {
            cache.set(response.clone()).await;
        }
        Ok(response)
    }

    /// GET `/supported` with 429-only retries, bypassing the TTL cache.
    ///
    /// # Errors
    ///
    /// Returns [`FacilitatorError`] if the HTTP request fails or the body is
    /// not a well-formed [`SupportedResponse`].
    pub async fn supported_inner(&self) -> Result<SupportedResponse, FacilitatorError> {
        let headers = self.auth_headers("supported").await?;
        for attempt in 0..GET_SUPPORTED_RETRIES {
            let raw = self
                .send(self.client.get(self.supported_url.clone()), &headers)
                .await?;
            if let Some(delay) =
                supported_retry_delay(raw.status, raw.retry_after.as_deref(), attempt)
            {
                #[cfg(feature = "telemetry")]
                tracing::warn!(
                    attempt,
                    delay_ms = delay.as_millis(),
                    status = raw.status.as_u16(),
                    "x402.facilitator_client.supported_retry",
                );
                tokio::time::sleep(delay).await;
                continue;
            }
            return parse_supported_body(&raw);
        }
        Err(FacilitatorError::transport(
            FacilitatorTransportKind::HttpStatus { status: 429 },
        ))
    }

    async fn auth_headers(&self, path: &'static str) -> Result<HeaderMap, FacilitatorError> {
        self.create_auth_headers(path)
            .await
            .map_err(FacilitatorError::internal)
    }

    async fn send(
        &self,
        mut req: reqwest::RequestBuilder,
        headers: &HeaderMap,
    ) -> Result<RawHttpResponse, FacilitatorError> {
        for (key, value) in headers {
            req = req.header(key, value);
        }
        if let Some(timeout) = self.timeout {
            req = req.timeout(timeout);
        }
        let http_response = req.send().await.map_err(map_reqwest)?;
        let retry_after = http_response
            .headers()
            .get(http::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let status = http_response.status();
        let response_headers = http_response.headers().clone();
        let body = http_response.bytes().await.map_err(map_reqwest)?.to_vec();
        Ok(RawHttpResponse {
            status,
            body,
            headers: response_headers,
            retry_after,
        })
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

fn join_endpoint(base: &Url, path: &str) -> Result<Url, FacilitatorClientError> {
    base.join(path)
        .map_err(|e| FacilitatorClientError::UrlParse {
            context: "Failed to construct facilitator endpoint URL",
            source: e,
        })
}

fn map_reqwest(err: reqwest::Error) -> FacilitatorError {
    if err.is_timeout() {
        FacilitatorError::transport(FacilitatorTransportKind::Timeout)
    } else {
        FacilitatorError::internal(err)
    }
}

fn parse_operation_body<T, F>(raw: &RawHttpResponse, attach: F) -> Result<T, FacilitatorError>
where
    T: DeserializeOwned,
    F: FnOnce(T, &HeaderMap) -> T,
{
    match serde_json::from_slice::<T>(&raw.body) {
        Ok(parsed) => Ok(attach(parsed, &raw.headers)),
        Err(_) if raw.status.is_success() => Err(FacilitatorError::transport(
            FacilitatorTransportKind::MalformedSuccessBody,
        )),
        Err(_) => Err(FacilitatorError::transport(
            FacilitatorTransportKind::HttpStatus {
                status: raw.status.as_u16(),
            },
        )),
    }
}

fn parse_supported_body(raw: &RawHttpResponse) -> Result<SupportedResponse, FacilitatorError> {
    if raw.status.is_success() {
        serde_json::from_slice(&raw.body).map_err(|_| {
            FacilitatorError::transport(FacilitatorTransportKind::MalformedSuccessBody)
        })
    } else {
        Err(FacilitatorError::transport(
            FacilitatorTransportKind::HttpStatus {
                status: raw.status.as_u16(),
            },
        ))
    }
}

fn attach_verify(mut response: VerifyResponse, headers: &HeaderMap) -> VerifyResponse {
    let side = parse_extension_responses(headers);
    log_extension_responses(&side);
    response.set_extension_responses(side);
    response
}

fn attach_settle(mut response: SettleResponse, headers: &HeaderMap) -> SettleResponse {
    let side = parse_extension_responses(headers);
    log_extension_responses(&side);
    response.set_extension_responses(side);
    response
}

fn parse_extension_responses(headers: &HeaderMap) -> Extensions {
    let Some(value) = headers.get("extension-responses") else {
        return Extensions::new();
    };
    let Ok(raw) = value.to_str() else {
        return Extensions::new();
    };
    decode_extension_responses(raw).unwrap_or_default()
}

/// Decode failure is ignored: missing, bad base64, non-object JSON, or
/// deserialize error all yield an empty map.
fn decode_extension_responses(header: &str) -> Option<Extensions> {
    let bytes = Base64Bytes(header.as_bytes().to_vec());
    let decoded = bytes.decode().ok()?;
    let value: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    if !value.is_object() {
        return None;
    }
    serde_json::from_value(value).ok()
}

fn log_extension_responses(responses: &Extensions) {
    #[cfg(feature = "telemetry")]
    {
        if responses.is_empty() {
            return;
        }
        let sanitized: serde_json::Map<String, serde_json::Value> = responses
            .iter()
            .map(|(key, entry)| {
                (
                    key.to_string(),
                    serde_json::Value::Object(allowlisted(entry)),
                )
            })
            .collect();
        tracing::info!(
            extension_responses = %serde_json::Value::Object(sanitized),
            "x402.facilitator_client.extension_responses"
        );
    }
    #[cfg(not(feature = "telemetry"))]
    let _ = responses;
}

#[cfg(feature = "telemetry")]
fn allowlisted(
    entry: &r402_protocol::payment::ExtensionEntry,
) -> serde_json::Map<String, serde_json::Value> {
    use r402_protocol::payment::EXTENSION_RESPONSE_LOG_FIELDS;
    let Some(obj) = entry.to_value().as_object().cloned() else {
        return serde_json::Map::new();
    };
    EXTENSION_RESPONSE_LOG_FIELDS
        .iter()
        .filter_map(|field| obj.get(*field).cloned().map(|v| ((*field).to_owned(), v)))
        .collect()
}

impl Facilitator for FacilitatorClient {
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        #[cfg(feature = "telemetry")]
        let result = with_span(
            Self::verify(self, &request),
            tracing::info_span!("x402.facilitator_client.verify", timeout = ?self.timeout),
        )
        .await;
        #[cfg(not(feature = "telemetry"))]
        let result = Self::verify(self, &request).await;
        result
    }

    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        #[cfg(feature = "telemetry")]
        let result = with_span(
            Self::settle(self, &request),
            tracing::info_span!("x402.facilitator_client.settle", timeout = ?self.timeout),
        )
        .await;
        #[cfg(not(feature = "telemetry"))]
        let result = Self::settle(self, &request).await;
        result
    }

    async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
        Self::supported(self).await
    }
}

impl TryFrom<&str> for FacilitatorClient {
    type Error = FacilitatorClientError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let mut normalized = value.trim_end_matches('/').to_owned();
        normalized.push('/');
        let url = Url::parse(&normalized).map_err(|e| FacilitatorClientError::UrlParse {
            context: "Failed to parse base url",
            source: e,
        })?;
        Self::try_new(url)
    }
}

impl TryFrom<String> for FacilitatorClient {
    type Error = FacilitatorClientError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

#[cfg(feature = "telemetry")]
fn with_span<F: Future>(fut: F, span: tracing::Span) -> impl Future<Output = F::Output> {
    use tracing::Instrument;
    fut.instrument(span)
}
