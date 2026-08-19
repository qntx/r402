//! A [`r402_core::facilitator::Facilitator`] implementation that interacts with a _remote_ x402 Facilitator over HTTP.
//!
//! This [`FacilitatorClient`] handles the `/verify`, `/settle`, and `/supported` endpoints of a remote facilitator,
//! and implements the [`r402_core::facilitator::Facilitator`] trait for compatibility
//! with x402-based middleware and logic.
//!
//! ## Features
//!
//! - Uses `reqwest` for async HTTP requests
//! - Supports optional timeout and headers
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
use std::sync::Arc;
use std::time::Duration;

use http::{HeaderMap, StatusCode};
use r402_core::facilitator::{Facilitator, FacilitatorError};
use r402_core::wire::{
    SettleRequest, SettleResponse, SupportedResponse, VerifyRequest, VerifyResponse,
};
use reqwest::Client;
use tokio::sync::RwLock;
#[cfg(feature = "telemetry")]
use tracing::{Instrument, Span, instrument};
use url::Url;

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
    /// Optional custom headers sent with each request
    headers: HeaderMap,
    /// Optional request timeout
    timeout: Option<Duration>,
    /// Cache for the supported endpoint response
    supported_cache: SupportedCache,
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
}

impl FacilitatorClient {
    /// Default TTL for caching the supported endpoint response (10 minutes).
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

    /// Returns any custom headers configured on the client.
    #[must_use]
    pub const fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Returns the configured timeout, if any.
    #[must_use]
    pub const fn timeout(&self) -> Option<&Duration> {
        self.timeout.as_ref()
    }

    /// Returns a reference to the supported cache.
    #[must_use]
    pub const fn supported_cache(&self) -> &SupportedCache {
        &self.supported_cache
    }

    /// Constructs a new [`FacilitatorClient`] from a base URL.
    ///
    /// This sets up `./verify`, `./settle`, and `./supported` endpoint URLs relative to the base.
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
            headers: HeaderMap::new(),
            timeout: Some(Self::DEFAULT_TIMEOUT),
            supported_cache: SupportedCache::new(Self::DEFAULT_SUPPORTED_CACHE_TTL),
        })
    }

    /// Attaches custom headers to all future requests.
    #[must_use]
    pub fn with_headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }

    /// Sets a timeout for all future requests.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets the TTL for caching the supported endpoint response.
    ///
    /// Default is 10 minutes. Use [`Self::without_supported_cache()`] to disable caching.
    #[must_use]
    pub fn with_supported_cache_ttl(mut self, ttl: Duration) -> Self {
        self.supported_cache = SupportedCache::new(ttl);
        self
    }

    /// Disables caching for the supported endpoint.
    #[must_use]
    pub fn without_supported_cache(self) -> Self {
        self.with_supported_cache_ttl(Duration::ZERO)
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
        self.post_json(&self.verify_url, "POST /verify", request)
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
        self.post_json(&self.settle_url, "POST /settle", request)
            .await
    }

    /// Sends a `GET /supported` request to the facilitator.
    /// This is the inner method that always makes an HTTP request.
    #[cfg_attr(
        feature = "telemetry",
        instrument(name = "x402.facilitator_client.supported", skip_all, err)
    )]
    async fn supported_inner(&self) -> Result<SupportedResponse, FacilitatorClientError> {
        // F-047: retry rate-limited (`429 Too Many Requests`) and transient
        // 5xx responses with exponential backoff. The supported endpoint is
        // idempotent and cached, so retrying is safe and limits flapping
        // when a facilitator is briefly overloaded.
        const MAX_ATTEMPTS: u32 = 3;
        let mut attempt: u32 = 0;
        loop {
            let result = self.get_json(&self.supported_url, "GET /supported").await;
            match result {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    let retriable = matches!(
                        &err,
                        FacilitatorClientError::HttpStatus { status, .. }
                            if *status == StatusCode::TOO_MANY_REQUESTS
                                || status.is_server_error()
                    );
                    attempt += 1;
                    if !retriable || attempt >= MAX_ATTEMPTS {
                        return Err(err);
                    }
                    let backoff_ms = 200_u64 << attempt;
                    let backoff = Duration::from_millis(backoff_ms);
                    #[cfg(feature = "telemetry")]
                    tracing::warn!(
                        attempt,
                        backoff_ms,
                        error = %err,
                        "x402.facilitator_client.supported_retry",
                    );
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }

    /// Sends a `GET /supported` request to the facilitator.
    /// Results are cached with a configurable TTL (default: 10 minutes).
    /// Use `supported_inner()` to bypass the cache.
    ///
    /// # Errors
    ///
    /// Returns [`FacilitatorClientError`] if the HTTP request fails.
    pub async fn supported(&self) -> Result<SupportedResponse, FacilitatorClientError> {
        // Try to get from cache
        if let Some(response) = self.supported_cache.get().await {
            return Ok(response);
        }

        // Cache miss - fetch and cache
        #[cfg(feature = "telemetry")]
        tracing::info!("x402.facilitator_client.supported_cache_miss");

        let response = self.supported_inner().await?;
        self.supported_cache.set(response.clone()).await;

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
    ) -> Result<R, FacilitatorClientError>
    where
        T: serde::Serialize + Sync + ?Sized,
        R: serde::de::DeserializeOwned,
    {
        let req = self.client.post(url.clone()).json(payload);
        self.send_and_parse(req, context).await
    }

    /// Generic GET helper that handles error mapping, timeout application,
    /// and telemetry integration.
    ///
    /// `context` is a human-readable identifier used in tracing and error messages (e.g. `"GET /supported"`).
    async fn get_json<R>(
        &self,
        url: &Url,
        context: &'static str,
    ) -> Result<R, FacilitatorClientError>
    where
        R: serde::de::DeserializeOwned,
    {
        let req = self.client.get(url.clone());
        self.send_and_parse(req, context).await
    }

    /// Applies headers, timeout, sends the request, and parses the JSON response.
    async fn send_and_parse<R>(
        &self,
        mut req: reqwest::RequestBuilder,
        context: &'static str,
    ) -> Result<R, FacilitatorClientError>
    where
        R: serde::de::DeserializeOwned,
    {
        for (key, value) in &self.headers {
            req = req.header(key, value);
        }
        if let Some(timeout) = self.timeout {
            req = req.timeout(timeout);
        }
        let http_response = req
            .send()
            .await
            .map_err(|e| FacilitatorClientError::Http { context, source: e })?;

        // F-044: facilitators MAY return structured `VerifyResponse::Invalid`
        // or `SettleResponse::Failure` bodies on non-2xx HTTP statuses
        // (e.g. 402, 412 per x402 v2 §HTTP transport error mapping). We
        // therefore always attempt to parse the body as the expected `R`,
        // falling back to `HttpStatus` only when the body cannot be parsed.
        let status = http_response.status();
        let body_bytes = http_response
            .bytes()
            .await
            .map_err(|e| FacilitatorClientError::ResponseBodyRead { context, source: e })?;

        let result = match serde_json::from_slice::<R>(&body_bytes) {
            Ok(parsed) => Ok(parsed),
            Err(parse_err) => {
                let body = String::from_utf8_lossy(&body_bytes).into_owned();
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
                    })
                }
            }
        };

        record_result_on_span(&result);

        result
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
    reason = "test assertions with known-length slices"
)]
mod tests {
    use r402_core::wire::SupportedPaymentKind;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

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

    fn create_test_supported_response() -> SupportedResponse {
        SupportedResponse::new().with_kinds(vec![SupportedPaymentKind::new(1, "eip155-exact", "1")])
    }

    #[tokio::test]
    async fn test_supported_cache_caches_response() {
        let mock_server = MockServer::start().await;
        let test_response = create_test_supported_response();

        // Mock the supported endpoint
        Mock::given(method("GET"))
            .and(path("/supported"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&test_response))
            .mount(&mock_server)
            .await;

        let client = FacilitatorClient::try_new(mock_server.uri().parse::<Url>().unwrap()).unwrap();

        // First call should hit the network
        let result1 = client.supported().await.unwrap();
        assert_eq!(result1.kinds.len(), 1);

        // Second call should use cache (same mock call count)
        let result2 = client.supported().await.unwrap();
        assert_eq!(result2.kinds.len(), 1);

        // Both results should be equal
        assert_eq!(result1.kinds[0].scheme, result2.kinds[0].scheme);
    }

    #[tokio::test]
    async fn test_supported_cache_with_custom_ttl() {
        let mock_server = MockServer::start().await;
        let test_response = create_test_supported_response();

        // Mock the supported endpoint
        Mock::given(method("GET"))
            .and(path("/supported"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&test_response))
            .mount(&mock_server)
            .await;

        // Create client with 1ms TTL (essentially no caching)
        let client = FacilitatorClient::try_new(mock_server.uri().parse::<Url>().unwrap())
            .unwrap()
            .with_supported_cache_ttl(Duration::from_millis(1));

        // First call
        let result1 = client.supported().await.unwrap();
        assert_eq!(result1.kinds.len(), 1);

        // Wait for cache to expire
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Second call should hit the network again due to expired cache
        let result2 = client.supported().await.unwrap();
        assert_eq!(result2.kinds.len(), 1);
    }

    #[tokio::test]
    async fn test_supported_cache_disabled() {
        let mock_server = MockServer::start().await;
        let test_response = create_test_supported_response();

        // Mock the supported endpoint
        Mock::given(method("GET"))
            .and(path("/supported"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&test_response))
            .mount(&mock_server)
            .await;

        // Create client with caching disabled
        let client = FacilitatorClient::try_new(mock_server.uri().parse::<Url>().unwrap())
            .unwrap()
            .without_supported_cache();

        // Each call should hit the network
        let result1 = client.supported().await.unwrap();
        let result2 = client.supported().await.unwrap();

        assert_eq!(result1.kinds.len(), 1);
        assert_eq!(result2.kinds.len(), 1);
    }

    #[tokio::test]
    async fn test_supported_cache_shared_across_clones() {
        let mock_server = MockServer::start().await;
        let test_response = create_test_supported_response();

        // Mock the supported endpoint — expect exactly 1 request
        Mock::given(method("GET"))
            .and(path("/supported"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&test_response))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = FacilitatorClient::try_new(mock_server.uri().parse::<Url>().unwrap()).unwrap();

        // Clone the client — clones share the same cache
        let client2 = client.clone();

        // Populate cache on first client
        let result1 = client.supported().await.unwrap();
        assert_eq!(result1.kinds.len(), 1);

        // Clone should hit the shared cache (no extra HTTP request)
        let result2 = client2.supported().await.unwrap();
        assert_eq!(result2.kinds.len(), 1);
        assert_eq!(result1.kinds[0].scheme, result2.kinds[0].scheme);
    }

    #[tokio::test]
    async fn test_supported_inner_bypasses_cache() {
        let mock_server = MockServer::start().await;
        let test_response = create_test_supported_response();

        // Mock the supported endpoint
        Mock::given(method("GET"))
            .and(path("/supported"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&test_response))
            .mount(&mock_server)
            .await;

        let client = FacilitatorClient::try_new(mock_server.uri().parse::<Url>().unwrap()).unwrap();

        // Populate cache
        let _ = client.supported().await.unwrap();

        // supported_inner() should always make HTTP request, bypassing cache
        let result = client.supported_inner().await.unwrap();
        assert_eq!(result.kinds.len(), 1);
    }
}
