//! HTTP client for a remote x402 facilitator.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use http::{HeaderMap, StatusCode};
use r402_protocol::error::{FacilitatorError, FacilitatorTransportKind};
use r402_protocol::payment::{
    SettleRequest, SettleResponse, SupportedResponse, VerifyRequest, VerifyResponse,
};
use reqwest::Client;
use url::Url;

use super::auth::{CreateAuthHeadersCallback, FacilitatorAuthHeaders};
use super::cache::SupportedCache;
use super::extension::{attach_settle, attach_verify};
use super::retry::supported_retry_delay;
use crate::Facilitator;

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
    /// Default `/supported` cache TTL used by [`Self::try_new`].
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
    /// `/supported` is cached for [`Self::DEFAULT_SUPPORTED_CACHE_TTL`].
    /// Call [`Self::without_supported_cache`] to disable.
    ///
    /// # Errors
    ///
    /// Returns [`FacilitatorClientError`] if URL construction fails.
    pub fn try_new(mut base_url: Url) -> Result<Self, FacilitatorClientError> {
        ensure_trailing_slash(&mut base_url);
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
            supported_cache: Some(SupportedCache::new(Self::DEFAULT_SUPPORTED_CACHE_TTL)),
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
    /// 2xx or non-2xx `Invalid` JSON is `Ok`. Non-2xx `Valid` JSON, timeout,
    /// connect/DNS/TLS/body failures, and malformed 2xx bodies are
    /// [`FacilitatorError::Transport`] (502).
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
        parse_verify_body(&raw)
    }

    /// Sends a `POST /settle` request to the facilitator.
    ///
    /// 2xx `Success` may have `transaction: ""`. Non-2xx `Failure` is `Ok`.
    /// Non-2xx `Success` is Transport.
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
        parse_settle_body(&raw)
    }

    /// Sends a `GET /supported` request to the facilitator.
    /// Results are cached when a TTL is configured (the default).
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
        let mut raw = self
            .send(self.client.get(self.supported_url.clone()), &headers)
            .await?;
        let mut attempt = 0;
        while let Some(delay) =
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
            attempt += 1;
            raw = self
                .send(self.client.get(self.supported_url.clone()), &headers)
                .await?;
        }
        parse_supported_body(&raw)
    }

    async fn auth_headers(&self, path: &'static str) -> Result<HeaderMap, FacilitatorError> {
        self.create_auth_headers(path)
            .await
            .map_err(|_| FacilitatorError::transport(FacilitatorTransportKind::Io))
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
        let http_response = req.send().await.map_err(|e| map_reqwest(&e))?;
        let retry_after = http_response
            .headers()
            .get(http::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let status = http_response.status();
        let response_headers = http_response.headers().clone();
        let body = http_response
            .bytes()
            .await
            .map_err(|e| map_reqwest(&e))?
            .to_vec();
        Ok(RawHttpResponse {
            status,
            body,
            headers: response_headers,
            retry_after,
        })
    }
}

/// `Url::join("./verify")` replaces the last path segment unless the base ends
/// with `/` (`https://x402.org/facilitator` would become `/verify`).
fn ensure_trailing_slash(url: &mut Url) {
    let path = url.path();
    if path.ends_with('/') {
        return;
    }
    let mut with_slash = path.to_owned();
    with_slash.push('/');
    url.set_path(&with_slash);
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

fn map_reqwest(err: &reqwest::Error) -> FacilitatorError {
    if err.is_timeout() {
        FacilitatorError::transport(FacilitatorTransportKind::Timeout)
    } else {
        FacilitatorError::transport(FacilitatorTransportKind::Io)
    }
}

const fn status_transport(status: StatusCode) -> FacilitatorError {
    FacilitatorError::transport(FacilitatorTransportKind::HttpStatus {
        status: status.as_u16(),
    })
}

fn parse_verify_body(raw: &RawHttpResponse) -> Result<VerifyResponse, FacilitatorError> {
    let parsed = match serde_json::from_slice::<VerifyResponse>(&raw.body) {
        Ok(parsed) => parsed,
        Err(_) if raw.status.is_success() => {
            return Err(FacilitatorError::transport(
                FacilitatorTransportKind::MalformedSuccessBody,
            ));
        }
        Err(_) => return Err(status_transport(raw.status)),
    };
    if raw.status.is_success() || !parsed.is_valid() {
        return Ok(attach_verify(parsed, &raw.headers));
    }
    Err(status_transport(raw.status))
}

fn parse_settle_body(raw: &RawHttpResponse) -> Result<SettleResponse, FacilitatorError> {
    let parsed = match serde_json::from_slice::<SettleResponse>(&raw.body) {
        Ok(parsed) => parsed,
        Err(_) if raw.status.is_success() => {
            return Err(FacilitatorError::transport(
                FacilitatorTransportKind::MalformedSuccessBody,
            ));
        }
        Err(_) => return Err(status_transport(raw.status)),
    };
    if raw.status.is_success() {
        return Ok(attach_settle(parsed, &raw.headers));
    }
    if parsed.is_success() {
        return Err(status_transport(raw.status));
    }
    Ok(attach_settle(parsed, &raw.headers))
}

fn parse_supported_body(raw: &RawHttpResponse) -> Result<SupportedResponse, FacilitatorError> {
    if raw.status.is_success() {
        serde_json::from_slice(&raw.body).map_err(|_| {
            FacilitatorError::transport(FacilitatorTransportKind::MalformedSuccessBody)
        })
    } else {
        Err(status_transport(raw.status))
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
        let url = Url::parse(value).map_err(|e| FacilitatorClientError::UrlParse {
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
