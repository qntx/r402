//! A [`r402_core::facilitator::Facilitator`] implementation that interacts with a _remote_ x402 Facilitator over HTTP.
//!
//! This [`FacilitatorClient`] handles the `/verify`, `/settle`, and `/supported` endpoints of a remote facilitator,
//! and implements the [`r402_core::facilitator::Facilitator`] trait for compatibility
//! with x402-based middleware and logic.
//!
//! ## Features
//!
//! - Uses `reqwest` for async HTTP requests
//! - Supports optional timeout and path-keyed auth headers
//! - Integrates with `tracing` if the `telemetry` feature is enabled
//!
//! ## Error Handling
//!
//! Custom error types capture detailed failure contexts, including
//! - URL construction
//! - HTTP transport failures
//! - JSON deserialization errors
//! - Unexpected HTTP status responses
//!

use std::fmt::Display;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use r402_core::error::FacilitatorError;
use r402_core::facilitator::Facilitator;
use r402_core::wire::{
    SettleRequest, SettleResponse, SupportedResponse, VerifyRequest, VerifyResponse,
};
use reqwest::Client;
use tokio::sync::RwLock;
#[cfg(feature = "telemetry")]
use tracing::{Instrument, Span, instrument};
use url::Url;

/// Number of `/supported` attempts, matching official `GET_SUPPORTED_RETRIES`.
const GET_SUPPORTED_RETRIES: u32 = 3;
/// Base delay in ms for exponential backoff when `Retry-After` is unusable.
const GET_SUPPORTED_RETRY_DELAY_MS: u64 = 1000;
/// Upper bound on retry delay.
const MAX_RETRY_DELAY_MS: u64 = 30_000;

type BoxedAuthHeadersFuture = Pin<Box<dyn Future<Output = serde_json::Value> + Send>>;
type BoxedCreateAuthHeaders = dyn Fn() -> BoxedAuthHeadersFuture + Send + Sync;

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
    fn from_json(value: &serde_json::Value) -> Result<Self, FacilitatorClientError> {
        if looks_flat(value) {
            return Err(FacilitatorClientError::FlatAuthHeaders);
        }
        Ok(Self {
            verify: header_map_at(value, "verify"),
            settle: header_map_at(value, "settle"),
            supported: header_map_at(value, "supported"),
            bazaar: header_map_at(value, "bazaar"),
        })
    }

    fn for_path(&self, path: &str) -> HeaderMap {
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
struct CreateAuthHeadersCallback(Arc<BoxedCreateAuthHeaders>);

impl std::fmt::Debug for CreateAuthHeadersCallback {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CreateAuthHeaders")
    }
}

/// TTL cache for [`SupportedResponse`].
#[derive(Clone, Debug)]
struct SupportedCacheState {
    /// The cached response
    response: SupportedResponse,
    /// When the cache expires
    expires_at: std::time::Instant,
}

/// An encapsulated TTL cache for the `/supported` endpoint response.
///
/// Clones share the same cache state via `Arc`, so cached responses are
/// visible across all clones (e.g. when the middleware clones the
/// facilitator per-request).
#[derive(Debug, Clone)]
pub struct SupportedCache {
    /// TTL for the cache
    ttl: Duration,
    /// Shared cache state (`Arc<RwLock>` so clones hit the same cache)
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
///
/// Handles `/verify`, `/settle`, and `/supported` endpoints via JSON HTTP.
#[derive(Clone, Debug)]
pub struct FacilitatorClient {
    /// Base URL of the facilitator (e.g. `https://facilitator.example/`)
    base_url: Url,
    /// Full URL to `POST /verify` requests
    verify_url: Url,
    /// Full URL to `POST /settle` requests
    settle_url: Url,
    /// Full URL to `GET /supported` requests
    supported_url: Url,
    /// Shared Reqwest HTTP client
    client: Client,
    /// Optional request timeout
    timeout: Option<Duration>,
    /// Cache for the supported endpoint response. `None` means uncached.
    supported_cache: Option<SupportedCache>,
    /// Optional callback producing path-keyed auth headers.
    create_auth_headers: Option<CreateAuthHeadersCallback>,
}

/// Errors that can occur while interacting with a remote facilitator.
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
    /// HTTP transport error.
    #[error("HTTP error: {context}: {source}")]
    Http {
        /// Human-readable context.
        context: &'static str,
        /// The underlying reqwest error.
        #[source]
        source: reqwest::Error,
    },
    /// JSON deserialization error.
    #[error("Failed to deserialize JSON: {context}: {source}")]
    JsonDeserialization {
        /// Human-readable context.
        context: &'static str,
        /// The underlying serde error.
        #[source]
        source: serde_json::Error,
        /// The raw body that failed to parse, included for diagnostics.
        body: String,
    },
    /// Unexpected HTTP status code.
    #[error("Unexpected HTTP status {status}: {context}: {body}")]
    HttpStatus {
        /// Human-readable context.
        context: &'static str,
        /// The HTTP status code.
        status: StatusCode,
        /// The response body.
        body: String,
        /// Raw `Retry-After` header, if present.
        retry_after: Option<String>,
    },
    /// Failed to read response body.
    #[error("Failed to read response body as text: {context}: {source}")]
    ResponseBodyRead {
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
}

/// Delay before the next `/supported` retry.
///
/// Parses `Retry-After` as RFC 7231 delta-seconds or HTTP-date. Missing,
/// invalid, or non-positive values fall back to `1000 * 2^attempt` ms.
/// The result is capped at 30 seconds.
#[must_use]
pub fn compute_retry_delay(retry_after: Option<&str>, attempt: u32) -> Duration {
    let parsed_ms = retry_after.and_then(parse_retry_after_ms);
    let delay_ms = match parsed_ms {
        Some(millis) if millis > 0 => millis,
        _ => GET_SUPPORTED_RETRY_DELAY_MS.saturating_mul(2u64.saturating_pow(attempt)),
    };
    Duration::from_millis(delay_ms.min(MAX_RETRY_DELAY_MS))
}

fn parse_retry_after_ms(raw: &str) -> Option<u64> {
    let trimmed = raw.trim();
    if !trimmed.is_empty() && trimmed.as_bytes().iter().all(u8::is_ascii_digit) {
        let seconds: u64 = trimmed.parse().ok()?;
        return Some(seconds.saturating_mul(1000));
    }
    let retry_at = httpdate::parse_http_date(trimmed).ok()?;
    retry_at
        .duration_since(std::time::SystemTime::now())
        .ok()
        .map(|delay| u64::try_from(delay.as_millis()).unwrap_or(u64::MAX))
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

fn header_map_at(value: &serde_json::Value, path: &str) -> HeaderMap {
    match value.get(path) {
        Some(headers) if is_header_object(headers) => json_object_to_header_map(headers),
        _ => HeaderMap::new(),
    }
}

fn supported_retry_delay(err: &FacilitatorClientError, attempt: u32) -> Option<Duration> {
    match err {
        FacilitatorClientError::HttpStatus {
            status,
            retry_after,
            ..
        } if *status == StatusCode::TOO_MANY_REQUESTS && attempt + 1 < GET_SUPPORTED_RETRIES => {
            Some(compute_retry_delay(retry_after.as_deref(), attempt))
        }
        _ => None,
    }
}

fn json_object_to_header_map(value: &serde_json::Value) -> HeaderMap {
    let Some(obj) = value.as_object() else {
        return HeaderMap::new();
    };
    let mut headers = HeaderMap::new();
    for (key, val) in obj {
        let Some(val) = val.as_str() else {
            continue;
        };
        let Ok(name) = HeaderName::try_from(key.as_str()) else {
            continue;
        };
        let Ok(header_value) = HeaderValue::try_from(val) else {
            continue;
        };
        headers.insert(name, header_value);
    }
    headers
}

impl FacilitatorClient {
    /// Suggested TTL when opting into `/supported` caching.
    pub const DEFAULT_SUPPORTED_CACHE_TTL: Duration = Duration::from_mins(10);

    /// Default per-request timeout. Aligned with the Go reference SDK
    /// (`http.DefaultClient` plus a 30 s wrapper) so a hung remote
    /// facilitator does not block payment flows indefinitely. Override via
    /// [`Self::with_timeout`] for stricter SLOs or faster fail-over.
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

    /// Returns the base URL used by this client.
    #[must_use]
    pub const fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Returns the computed `./verify` URL relative to [`FacilitatorClient::base_url`].
    #[must_use]
    pub const fn verify_url(&self) -> &Url {
        &self.verify_url
    }

    /// Returns the computed `./settle` URL relative to [`FacilitatorClient::base_url`].
    #[must_use]
    pub const fn settle_url(&self) -> &Url {
        &self.settle_url
    }

    /// Returns the computed `./supported` URL relative to [`FacilitatorClient::base_url`].
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
    /// This sets up `./verify`, `./settle`, and `./supported` endpoint URLs relative to the base.
    /// `/supported` is uncached until [`Self::with_supported_cache_ttl`] is called.
    ///
    /// # Errors
    ///
    /// Returns [`FacilitatorClientError`] if URL construction fails.
    pub fn try_new(base_url: Url) -> Result<Self, FacilitatorClientError> {
        let client = Client::builder()
            .timeout(Self::DEFAULT_TIMEOUT)
            .build()
            .map_err(|e| FacilitatorClientError::Http {
                context: "failed to build reqwest client with default timeout",
                source: e,
            })?;
        let verify_url =
            base_url
                .join("./verify")
                .map_err(|e| FacilitatorClientError::UrlParse {
                    context: "Failed to construct ./verify URL",
                    source: e,
                })?;
        let settle_url =
            base_url
                .join("./settle")
                .map_err(|e| FacilitatorClientError::UrlParse {
                    context: "Failed to construct ./settle URL",
                    source: e,
                })?;
        let supported_url =
            base_url
                .join("./supported")
                .map_err(|e| FacilitatorClientError::UrlParse {
                    context: "Failed to construct ./supported URL",
                    source: e,
                })?;
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
        Fut: Future<Output = serde_json::Value> + Send + 'static,
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
    ///
    /// Caching is off by default. Use [`Self::without_supported_cache`] to
    /// disable it after enabling.
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
    /// and some value is not a header object.
    pub async fn create_auth_headers(
        &self,
        path: &str,
    ) -> Result<HeaderMap, FacilitatorClientError> {
        let Some(callback) = &self.create_auth_headers else {
            return Ok(HeaderMap::new());
        };
        let value = (callback.0)().await;
        let parsed = FacilitatorAuthHeaders::from_json(&value)?;
        Ok(parsed.for_path(path))
    }

    /// Sends a `POST /verify` request to the facilitator.
    ///
    /// # Errors
    ///
    /// Returns [`FacilitatorClientError`] if the HTTP request fails.
    pub async fn verify(
        &self,
        request: &VerifyRequest,
    ) -> Result<VerifyResponse, FacilitatorClientError> {
        let headers = self.create_auth_headers("verify").await?;
        self.post_json(&self.verify_url, "POST /verify", request, &headers)
            .await
    }

    /// Sends a `POST /settle` request to the facilitator.
    ///
    /// # Errors
    ///
    /// Returns [`FacilitatorClientError`] if the HTTP request fails.
    pub async fn settle(
        &self,
        request: &SettleRequest,
    ) -> Result<SettleResponse, FacilitatorClientError> {
        let headers = self.create_auth_headers("settle").await?;
        self.post_json(&self.settle_url, "POST /settle", request, &headers)
            .await
    }

    /// Sends a `GET /supported` request to the facilitator.
    /// Retries HTTP 429 only, honoring `Retry-After` when present.
    #[cfg_attr(
        feature = "telemetry",
        instrument(name = "x402.facilitator_client.supported", skip_all, err)
    )]
    async fn supported_inner(&self) -> Result<SupportedResponse, FacilitatorClientError> {
        let headers = self.create_auth_headers("supported").await?;
        for attempt in 0..GET_SUPPORTED_RETRIES {
            match self
                .get_json(&self.supported_url, "GET /supported", &headers)
                .await
            {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    let Some(delay) = supported_retry_delay(&err, attempt) else {
                        return Err(err);
                    };
                    #[cfg(feature = "telemetry")]
                    tracing::warn!(
                        attempt,
                        delay_ms = delay.as_millis(),
                        error = %err,
                        "x402.facilitator_client.supported_retry",
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }
        Err(FacilitatorClientError::HttpStatus {
            context: "GET /supported",
            status: StatusCode::TOO_MANY_REQUESTS,
            body: "Facilitator getSupported failed after retries".to_owned(),
            retry_after: None,
        })
    }

    /// Sends a `GET /supported` request to the facilitator.
    /// Results are cached only when a TTL was configured.
    ///
    /// # Errors
    ///
    /// Returns [`FacilitatorClientError`] if the HTTP request fails.
    pub async fn supported(&self) -> Result<SupportedResponse, FacilitatorClientError> {
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

    /// Generic POST helper that handles JSON serialization, error mapping,
    /// timeout application, and telemetry integration.
    ///
    /// `context` is a human-readable identifier used in tracing and error messages (e.g. `"POST /verify"`).
    #[allow(
        clippy::needless_pass_by_value,
        reason = "context is a static str, clone cost is zero"
    )]
    async fn post_json<T, R>(
        &self,
        url: &Url,
        context: &'static str,
        payload: &T,
        headers: &HeaderMap,
    ) -> Result<R, FacilitatorClientError>
    where
        T: serde::Serialize + Sync + ?Sized,
        R: serde::de::DeserializeOwned,
    {
        let req = self.client.post(url.clone()).json(payload);
        self.send_and_parse(req, context, headers).await
    }

    /// Generic GET helper that handles error mapping, timeout application,
    /// and telemetry integration.
    ///
    /// `context` is a human-readable identifier used in tracing and error messages (e.g. `"GET /supported"`).
    async fn get_json<R>(
        &self,
        url: &Url,
        context: &'static str,
        headers: &HeaderMap,
    ) -> Result<R, FacilitatorClientError>
    where
        R: serde::de::DeserializeOwned,
    {
        let req = self.client.get(url.clone());
        self.send_and_parse(req, context, headers).await
    }

    /// Applies headers, timeout, sends the request, and parses the JSON response.
    async fn send_and_parse<R>(
        &self,
        mut req: reqwest::RequestBuilder,
        context: &'static str,
        headers: &HeaderMap,
    ) -> Result<R, FacilitatorClientError>
    where
        R: serde::de::DeserializeOwned,
    {
        for (key, value) in headers {
            req = req.header(key, value);
        }
        if let Some(timeout) = self.timeout {
            req = req.timeout(timeout);
        }
        let http_response = req
            .send()
            .await
            .map_err(|e| FacilitatorClientError::Http { context, source: e })?;

        let retry_after = http_response
            .headers()
            .get(http::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);

        let status = http_response.status();
        let body_bytes = http_response
            .bytes()
            .await
            .map_err(|e| FacilitatorClientError::ResponseBodyRead { context, source: e })?;

        let result = parse_facilitator_body(status, &body_bytes, context, retry_after);
        record_result_on_span(&result);
        result
    }
}

fn parse_facilitator_body<R>(
    status: StatusCode,
    body_bytes: &[u8],
    context: &'static str,
    retry_after: Option<String>,
) -> Result<R, FacilitatorClientError>
where
    R: serde::de::DeserializeOwned,
{
    match serde_json::from_slice::<R>(body_bytes) {
        Ok(parsed) => Ok(parsed),
        Err(parse_err) => {
            let body = String::from_utf8_lossy(body_bytes).into_owned();
            if status.is_success() {
                Err(FacilitatorClientError::JsonDeserialization {
                    context,
                    source: parse_err,
                    body,
                })
            } else {
                Err(FacilitatorClientError::HttpStatus {
                    context,
                    status,
                    body,
                    retry_after,
                })
            }
        }
    }
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
        result.map_err(|e| FacilitatorError::Internal(Box::new(e)))
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
        result.map_err(|e| FacilitatorError::Internal(Box::new(e)))
    }

    async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
        Self::supported(self)
            .await
            .map_err(|e| FacilitatorError::Internal(Box::new(e)))
    }
}

/// Converts a string URL into a `FacilitatorClient`, parsing the URL and calling `try_new`.
impl TryFrom<&str> for FacilitatorClient {
    type Error = FacilitatorClientError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        // Normalize: strip trailing slashes and add a single trailing slash
        let mut normalized = value.trim_end_matches('/').to_owned();
        normalized.push('/');
        let url = Url::parse(&normalized).map_err(|e| FacilitatorClientError::UrlParse {
            context: "Failed to parse base url",
            source: e,
        })?;
        Self::try_new(url)
    }
}

/// Converts a String URL into a `FacilitatorClient`.
impl TryFrom<String> for FacilitatorClient {
    type Error = FacilitatorClientError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

/// Records the outcome of a request on a tracing span, including status and errors.
#[cfg(feature = "telemetry")]
fn record_result_on_span<R, E: Display>(result: &Result<R, E>) {
    let span = Span::current();
    match result {
        Ok(_) => {
            span.record("otel.status_code", "OK");
        }
        Err(err) => {
            span.record("otel.status_code", "ERROR");
            span.record("error.message", tracing::field::display(err));
            tracing::event!(tracing::Level::ERROR, error = %err, "Request to facilitator failed");
        }
    }
}

/// Records the outcome of a request on a tracing span, including status and errors.
/// Noop if telemetry feature is off.
#[cfg(not(feature = "telemetry"))]
const fn record_result_on_span<R, E: Display>(_result: &Result<R, E>) {}

/// Instruments a future with a given tracing span.
#[cfg(feature = "telemetry")]
fn with_span<F: Future>(fut: F, span: Span) -> impl Future<Output = F::Output> {
    fut.instrument(span)
}

#[cfg(test)]
#[allow(
    clippy::indexing_slicing,
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test assertions with known-length slices"
)]
mod tests {
    use std::time::{Duration, SystemTime};

    use r402_core::wire::SupportedPaymentKind;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn test_client(uri: &str) -> FacilitatorClient {
        FacilitatorClient::try_new(uri.parse::<Url>().unwrap()).unwrap()
    }

    #[test]
    fn try_from_str_stores_normalized_base_url() {
        let client = FacilitatorClient::try_from("https://facilitator.example.com")
            .expect("valid facilitator URL");
        assert_eq!(
            client.base_url().as_str(),
            "https://facilitator.example.com/"
        );
        assert_eq!(
            client.verify_url().as_str(),
            "https://facilitator.example.com/verify"
        );
        assert_eq!(
            client.settle_url().as_str(),
            "https://facilitator.example.com/settle"
        );
        assert_eq!(
            client.supported_url().as_str(),
            "https://facilitator.example.com/supported"
        );
    }

    #[test]
    fn try_from_str_rejects_invalid_url() {
        let err = FacilitatorClient::try_from("not a url");
        assert!(
            err.is_err(),
            "invalid facilitator URL must return Err, not panic"
        );
        match err {
            Err(FacilitatorClientError::UrlParse { context, .. }) => {
                assert_eq!(context, "Failed to parse base url");
            }
            other => panic!("expected UrlParse, got {other:?}"),
        }
    }

    #[test]
    fn try_new_supported_cache_is_none() {
        let client = FacilitatorClient::try_from("https://facilitator.example.com")
            .expect("valid facilitator URL");
        assert!(
            client.supported_cache().is_none(),
            "try_new must not install a /supported cache"
        );
    }

    #[test]
    fn without_supported_cache_clears_opt_in_cache() {
        let client = FacilitatorClient::try_from("https://facilitator.example.com")
            .expect("valid facilitator URL")
            .with_supported_cache_ttl(Duration::from_mins(10));
        assert!(
            client.supported_cache().is_some(),
            "TTL builder must install a cache"
        );
        let client = client.without_supported_cache();
        assert!(
            client.supported_cache().is_none(),
            "without_supported_cache must store None, not a zero-TTL cache"
        );
    }

    fn create_test_supported_response() -> SupportedResponse {
        SupportedResponse::new().with_kinds(vec![SupportedPaymentKind::new(1, "eip155-exact", "1")])
    }

    #[tokio::test]
    async fn test_supported_cache_caches_response() {
        let mock_server = MockServer::start().await;
        let test_response = create_test_supported_response();

        Mock::given(method("GET"))
            .and(path("/supported"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&test_response))
            .expect(2)
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());

        let result1 = client.supported().await.unwrap();
        assert_eq!(result1.kinds.len(), 1);

        let result2 = client.supported().await.unwrap();
        assert_eq!(result2.kinds.len(), 1);
        assert_eq!(result1.kinds[0].scheme, result2.kinds[0].scheme);
    }

    #[tokio::test]
    async fn test_supported_cache_with_custom_ttl() {
        let mock_server = MockServer::start().await;
        let test_response = create_test_supported_response();

        Mock::given(method("GET"))
            .and(path("/supported"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&test_response))
            .mount(&mock_server)
            .await;

        let client =
            test_client(&mock_server.uri()).with_supported_cache_ttl(Duration::from_millis(1));

        let result1 = client.supported().await.unwrap();
        assert_eq!(result1.kinds.len(), 1);

        tokio::time::sleep(Duration::from_millis(10)).await;

        let result2 = client.supported().await.unwrap();
        assert_eq!(result2.kinds.len(), 1);
    }

    #[tokio::test]
    async fn test_supported_cache_disabled() {
        let mock_server = MockServer::start().await;
        let test_response = create_test_supported_response();

        Mock::given(method("GET"))
            .and(path("/supported"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&test_response))
            .expect(2)
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri())
            .with_supported_cache_ttl(Duration::from_mins(10))
            .without_supported_cache();

        let result1 = client.supported().await.unwrap();
        let result2 = client.supported().await.unwrap();

        assert_eq!(result1.kinds.len(), 1);
        assert_eq!(result2.kinds.len(), 1);
    }

    #[tokio::test]
    async fn test_supported_cache_shared_across_clones() {
        let mock_server = MockServer::start().await;
        let test_response = create_test_supported_response();

        Mock::given(method("GET"))
            .and(path("/supported"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&test_response))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client =
            test_client(&mock_server.uri()).with_supported_cache_ttl(Duration::from_mins(10));
        let client2 = client.clone();

        let result1 = client.supported().await.unwrap();
        assert_eq!(result1.kinds.len(), 1);

        let result2 = client2.supported().await.unwrap();
        assert_eq!(result2.kinds.len(), 1);
        assert_eq!(result1.kinds[0].scheme, result2.kinds[0].scheme);
    }

    #[tokio::test]
    async fn test_supported_inner_bypasses_cache() {
        let mock_server = MockServer::start().await;
        let test_response = create_test_supported_response();

        Mock::given(method("GET"))
            .and(path("/supported"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&test_response))
            .expect(2)
            .mount(&mock_server)
            .await;

        let client =
            test_client(&mock_server.uri()).with_supported_cache_ttl(Duration::from_mins(10));

        let _ = client.supported().await.unwrap();

        let result = client.supported_inner().await.unwrap();
        assert_eq!(result.kinds.len(), 1);
    }

    #[tokio::test]
    async fn create_auth_headers_returns_path_scoped_values() {
        let client = FacilitatorClient::try_from("https://facilitator.example.com")
            .unwrap()
            .with_auth(|| async {
                serde_json::json!({
                    "verify": { "Authorization": "Bearer verify" },
                    "settle": { "Authorization": "Bearer settle" },
                    "supported": { "Authorization": "Bearer supported" },
                })
            });

        let verify = client.create_auth_headers("verify").await.unwrap();
        assert_eq!(verify.get("Authorization").unwrap(), "Bearer verify");
        let settle = client.create_auth_headers("settle").await.unwrap();
        assert_eq!(settle.get("Authorization").unwrap(), "Bearer settle");
        let omitted = client.create_auth_headers("bazaar").await.unwrap();
        assert!(omitted.is_empty(), "omitted path must send no auth");
    }

    #[tokio::test]
    async fn create_auth_headers_empty_object_is_not_flat() {
        let client = FacilitatorClient::try_from("https://facilitator.example.com")
            .unwrap()
            .with_auth(|| async { serde_json::json!({}) });
        let headers = client.create_auth_headers("verify").await.unwrap();
        assert!(headers.is_empty(), "empty object yields no headers");
    }

    #[tokio::test]
    async fn create_auth_headers_looks_flat_rejects_authorization_string() {
        let client = FacilitatorClient::try_from("https://facilitator.example.com")
            .unwrap()
            .with_auth(|| async { serde_json::json!({ "Authorization": "Bearer token" }) });
        let err = client.create_auth_headers("verify").await.unwrap_err();
        assert!(
            matches!(err, FacilitatorClientError::FlatAuthHeaders),
            "flat Authorization string must error, got {err:?}"
        );
        assert!(
            err.to_string().contains("keyed by facilitator path"),
            "error must mention path keys, got {err}"
        );
    }

    #[tokio::test]
    async fn create_auth_headers_looks_flat_rejects_non_object_values() {
        let client = FacilitatorClient::try_from("https://facilitator.example.com")
            .unwrap()
            .with_auth(|| async { serde_json::json!({ "Authorization": 123 }) });
        let err = client.create_auth_headers("verify").await.unwrap_err();
        assert!(
            matches!(err, FacilitatorClientError::FlatAuthHeaders),
            "flat non-object values must error, got {err:?}"
        );
    }

    #[tokio::test]
    async fn create_auth_headers_nested_non_path_object_is_not_flat() {
        let client = FacilitatorClient::try_from("https://facilitator.example.com")
            .unwrap()
            .with_auth(|| async { serde_json::json!({ "X-Api-Key": { "nested": "1" } }) });
        let headers = client.create_auth_headers("verify").await.unwrap();
        assert!(
            headers.is_empty(),
            "no path key means no headers, but must not look flat"
        );
    }

    #[tokio::test]
    async fn create_auth_headers_bazaar_path_key_is_not_flat() {
        let client = FacilitatorClient::try_from("https://facilitator.example.com")
            .unwrap()
            .with_auth(|| async {
                serde_json::json!({ "bazaar": { "Authorization": "Bearer bazaar" } })
            });
        let verify = client.create_auth_headers("verify").await.unwrap();
        assert!(verify.is_empty(), "verify omitted on a bazaar-only object");
        let bazaar = client.create_auth_headers("bazaar").await.unwrap();
        assert_eq!(bazaar.get("Authorization").unwrap(), "Bearer bazaar");
    }

    #[tokio::test]
    async fn verify_sends_path_keyed_auth_headers() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/verify"))
            .and(header("authorization", "Bearer verify"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(VerifyResponse::valid("0xpayer")),
            )
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri()).with_auth(|| async {
            serde_json::json!({
                "verify": { "Authorization": "Bearer verify" },
                "settle": { "Authorization": "Bearer settle" },
                "supported": { "Authorization": "Bearer supported" },
            })
        });

        client
            .verify(&VerifyRequest::from(serde_json::json!({})))
            .await
            .expect("verify with path-keyed auth");
    }

    #[tokio::test]
    async fn supported_retries_429_honoring_retry_after() {
        let mock_server = MockServer::start().await;
        let test_response = create_test_supported_response();

        Mock::given(method("GET"))
            .and(path("/supported"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "2")
                    .set_body_string("rate limited"),
            )
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/supported"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&test_response))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let started = std::time::Instant::now();
        let result = client.supported().await.expect("retry then success");
        let elapsed = started.elapsed();
        assert_eq!(result.kinds.len(), 1);
        assert!(
            elapsed >= Duration::from_secs(2),
            "Retry-After of 2s must be honored, elapsed {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(4),
            "retry delay must not use a later backoff step, elapsed {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn supported_does_not_retry_5xx() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/supported"))
            .respond_with(ResponseTemplate::new(500).set_body_string("unavailable"))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let err = client.supported().await.unwrap_err();
        match err {
            FacilitatorClientError::HttpStatus { status, .. } => {
                assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            }
            other => panic!("expected HttpStatus 500, got {other:?}"),
        }
    }

    #[test]
    fn compute_retry_delay_delta_seconds() {
        assert_eq!(compute_retry_delay(Some("5"), 0), Duration::from_secs(5));
        assert_eq!(compute_retry_delay(Some("12"), 1), Duration::from_secs(12));
    }

    #[test]
    fn compute_retry_delay_http_date() {
        let future = SystemTime::now() + Duration::from_secs(7);
        let formatted = httpdate::fmt_http_date(future);
        let delay = compute_retry_delay(Some(&formatted), 0);
        assert!(
            delay > Duration::from_secs(5),
            "HTTP-date ~7s in the future, got {delay:?}"
        );
        assert!(
            delay <= Duration::from_secs(7),
            "HTTP-date ~7s in the future, got {delay:?}"
        );
    }

    #[test]
    fn compute_retry_delay_falls_back_to_exponential() {
        assert_eq!(compute_retry_delay(None, 0), Duration::from_secs(1));
        assert_eq!(compute_retry_delay(None, 1), Duration::from_secs(2));
        assert_eq!(compute_retry_delay(None, 2), Duration::from_secs(4));
        assert_eq!(compute_retry_delay(Some("0"), 0), Duration::from_secs(1));
        assert_eq!(compute_retry_delay(Some("-5"), 1), Duration::from_secs(2));
        assert_eq!(
            compute_retry_delay(Some("not-a-date"), 1),
            Duration::from_secs(2)
        );
        assert_eq!(compute_retry_delay(Some("1.5"), 1), Duration::from_secs(2));
        assert_eq!(
            compute_retry_delay(Some("9999"), 0),
            Duration::from_secs(30)
        );
    }
}
