//! HTTP-based facilitator client for the x402 protocol.
//!
//! Provides [`HttpFacilitatorClient`] which implements
//! [`r402::server::FacilitatorClient`] by making HTTP calls to a remote
//! facilitator service (e.g., the CDP facilitator at `x402.org`).
//!
//! Also provides [`AuthProvider`] for authenticated facilitator endpoints
//! (e.g., CDP API key authentication).
//!
//! Corresponds to Python SDK's `http/facilitator_client_base.py` +
//! `http/facilitator_client.py`.

use std::time::Duration;

use r402::proto::{
    PaymentPayload, PaymentRequirements, SettleResponse, SupportedResponse, VerifyResponse,
};
use r402::scheme::{BoxFuture, SchemeError};
use r402::server::FacilitatorClient;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::Serialize;

use crate::constants::DEFAULT_FACILITATOR_URL;

/// Per-endpoint authentication headers.
///
/// Corresponds to Python SDK's `AuthHeaders` in `facilitator_client_base.py`.
#[derive(Debug, Clone, Default)]
pub struct AuthHeaders {
    /// Headers to include in verify requests.
    pub verify: HeaderMap,
    /// Headers to include in settle requests.
    pub settle: HeaderMap,
    /// Headers to include in get-supported requests.
    pub supported: HeaderMap,
}

/// Generates authentication headers for facilitator requests.
///
/// Implement this trait to provide custom authentication (e.g., CDP API
/// keys, OAuth tokens) for facilitator endpoints.
///
/// Corresponds to Python SDK's `AuthProvider` protocol.
pub trait AuthProvider: Send + Sync {
    /// Returns authentication headers for each facilitator endpoint.
    fn get_auth_headers(&self) -> AuthHeaders;
}

/// [`AuthProvider`] that wraps a static set of headers applied to all endpoints.
///
/// Useful for simple API key authentication where the same header is sent
/// to all facilitator endpoints.
#[derive(Debug, Clone)]
pub struct StaticAuthProvider {
    headers: HeaderMap,
}

impl StaticAuthProvider {
    /// Creates a new provider that sends the same headers to all endpoints.
    #[must_use]
    pub fn new(headers: HeaderMap) -> Self {
        Self { headers }
    }

    /// Creates a provider from a single bearer token.
    ///
    /// # Panics
    ///
    /// Panics if `token` contains invalid header characters.
    #[must_use]
    pub fn bearer(token: &str) -> Self {
        let mut headers = HeaderMap::new();
        let value = HeaderValue::from_str(&format!("Bearer {token}")).expect("valid bearer token");
        headers.insert(reqwest::header::AUTHORIZATION, value);
        Self { headers }
    }
}

impl AuthProvider for StaticAuthProvider {
    fn get_auth_headers(&self) -> AuthHeaders {
        AuthHeaders {
            verify: self.headers.clone(),
            settle: self.headers.clone(),
            supported: self.headers.clone(),
        }
    }
}

/// [`AuthProvider`] that wraps a per-endpoint header map callback.
///
/// Adapts the dict-style `create_headers` function (as used by CDP SDK)
/// to the [`AuthProvider`] trait.
///
/// Corresponds to Python SDK's `CreateHeadersAuthProvider`.
pub struct CallbackAuthProvider<F> {
    create_headers: F,
}

impl<F> std::fmt::Debug for CallbackAuthProvider<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallbackAuthProvider")
            .finish_non_exhaustive()
    }
}

impl<F> CallbackAuthProvider<F>
where
    F: Fn() -> AuthHeaders + Send + Sync,
{
    /// Creates a new provider from a callback that returns [`AuthHeaders`].
    pub fn new(create_headers: F) -> Self {
        Self { create_headers }
    }
}

impl<F> AuthProvider for CallbackAuthProvider<F>
where
    F: Fn() -> AuthHeaders + Send + Sync,
{
    fn get_auth_headers(&self) -> AuthHeaders {
        (self.create_headers)()
    }
}

/// Configuration for [`HttpFacilitatorClient`].
///
/// Corresponds to Python SDK's `FacilitatorConfig` in
/// `facilitator_client_base.py`.
pub struct FacilitatorConfig {
    /// Facilitator service base URL (without trailing slash).
    pub url: String,

    /// HTTP request timeout.
    pub timeout: Duration,

    /// Optional authentication provider.
    pub auth_provider: Option<Box<dyn AuthProvider>>,

    /// Optional pre-configured reqwest client. If `None`, a new client is
    /// created with the configured timeout.
    pub http_client: Option<reqwest::Client>,

    /// Optional human-readable identifier for this facilitator
    /// (defaults to URL).
    pub identifier: Option<String>,
}

impl Default for FacilitatorConfig {
    fn default() -> Self {
        Self {
            url: DEFAULT_FACILITATOR_URL.to_owned(),
            timeout: Duration::from_secs(30),
            auth_provider: None,
            http_client: None,
            identifier: None,
        }
    }
}

impl FacilitatorConfig {
    /// Creates a config with the given facilitator URL.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Self::default()
        }
    }

    /// Sets the request timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sets the authentication provider.
    #[must_use]
    pub fn with_auth(mut self, provider: impl AuthProvider + 'static) -> Self {
        self.auth_provider = Some(Box::new(provider));
        self
    }

    /// Sets a pre-configured reqwest client.
    #[must_use]
    pub fn with_http_client(mut self, client: reqwest::Client) -> Self {
        self.http_client = Some(client);
        self
    }

    /// Sets the identifier.
    #[must_use]
    pub fn with_identifier(mut self, id: impl Into<String>) -> Self {
        self.identifier = Some(id.into());
        self
    }
}

impl std::fmt::Debug for FacilitatorConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FacilitatorConfig")
            .field("url", &self.url)
            .field("timeout", &self.timeout)
            .field("has_auth_provider", &self.auth_provider.is_some())
            .field("has_http_client", &self.http_client.is_some())
            .field("identifier", &self.identifier)
            .finish()
    }
}

/// Wire format for verify/settle request bodies sent to the facilitator.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FacilitatorRequestBody {
    x402_version: u32,
    payment_payload: serde_json::Value,
    payment_requirements: serde_json::Value,
}

/// Async HTTP-based facilitator client.
///
/// Communicates with a remote x402 facilitator service over HTTP.
/// Implements [`FacilitatorClient`] so it can be used with
/// [`r402::server::X402ResourceServer`].
///
/// # Example
///
/// ```no_run
/// use r402_http::facilitator::{HttpFacilitatorClient, FacilitatorConfig};
///
/// let client = HttpFacilitatorClient::new(FacilitatorConfig::default());
/// // Use with X402ResourceServer::with_facilitator(Box::new(client))
/// ```
///
/// Corresponds to Python SDK's `HTTPFacilitatorClient` in
/// `facilitator_client.py`.
pub struct HttpFacilitatorClient {
    url: String,
    identifier: String,
    auth_provider: Option<Box<dyn AuthProvider>>,
    client: reqwest::Client,
}

impl HttpFacilitatorClient {
    /// Creates a new HTTP facilitator client from the given configuration.
    pub fn new(config: FacilitatorConfig) -> Self {
        let url = config.url.trim_end_matches('/').to_owned();
        let identifier = config.identifier.unwrap_or_else(|| url.clone());

        let client = config.http_client.unwrap_or_else(|| {
            reqwest::Client::builder()
                .timeout(config.timeout)
                .redirect(reqwest::redirect::Policy::limited(10))
                .build()
                .expect("failed to build reqwest::Client")
        });

        Self {
            url,
            identifier,
            auth_provider: config.auth_provider,
            client,
        }
    }

    /// Creates a client with the default CDP facilitator URL.
    #[must_use]
    pub fn default_cdp() -> Self {
        Self::new(FacilitatorConfig::default())
    }

    /// Returns the facilitator base URL.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Returns the effective identifier.
    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// Builds headers for a verify request.
    fn verify_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(auth) = &self.auth_provider {
            let auth_headers = auth.get_auth_headers();
            headers.extend(auth_headers.verify);
        }
        headers
    }

    /// Builds headers for a settle request.
    fn settle_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(auth) = &self.auth_provider {
            let auth_headers = auth.get_auth_headers();
            headers.extend(auth_headers.settle);
        }
        headers
    }

    /// Builds headers for a get-supported request.
    fn supported_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(auth) = &self.auth_provider {
            let auth_headers = auth.get_auth_headers();
            headers.extend(auth_headers.supported);
        }
        headers
    }

    /// Builds the JSON request body for verify/settle.
    fn build_request_body(
        version: u32,
        payload: &serde_json::Value,
        requirements: &serde_json::Value,
    ) -> FacilitatorRequestBody {
        FacilitatorRequestBody {
            x402_version: version,
            payment_payload: payload.clone(),
            payment_requirements: requirements.clone(),
        }
    }

    /// Internal: POST to facilitator verify endpoint.
    async fn verify_http(
        &self,
        version: u32,
        payload_value: serde_json::Value,
        requirements_value: serde_json::Value,
    ) -> Result<VerifyResponse, SchemeError> {
        let body = Self::build_request_body(version, &payload_value, &requirements_value);

        let response = self
            .client
            .post(format!("{}/verify", self.url))
            .headers(self.verify_headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| -> SchemeError {
                format!("Facilitator verify request failed: {e}").into()
            })?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(format!("Facilitator verify failed ({status}): {text}").into());
        }

        let result: VerifyResponse = response.json().await.map_err(|e| -> SchemeError {
            format!("Facilitator verify response parse error: {e}").into()
        })?;

        Ok(result)
    }

    /// Internal: POST to facilitator settle endpoint.
    async fn settle_http(
        &self,
        version: u32,
        payload_value: serde_json::Value,
        requirements_value: serde_json::Value,
    ) -> Result<SettleResponse, SchemeError> {
        let body = Self::build_request_body(version, &payload_value, &requirements_value);

        let response = self
            .client
            .post(format!("{}/settle", self.url))
            .headers(self.settle_headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| -> SchemeError {
                format!("Facilitator settle request failed: {e}").into()
            })?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(format!("Facilitator settle failed ({status}): {text}").into());
        }

        let result: SettleResponse = response.json().await.map_err(|e| -> SchemeError {
            format!("Facilitator settle response parse error: {e}").into()
        })?;

        Ok(result)
    }

    /// Verifies a payment from raw JSON bytes.
    ///
    /// Operates at the network boundary — auto-detects protocol version.
    ///
    /// # Errors
    ///
    /// Returns an error on network failure, non-200 response, or parse error.
    pub async fn verify_from_bytes(
        &self,
        payload_bytes: &[u8],
        requirements_bytes: &[u8],
    ) -> Result<VerifyResponse, SchemeError> {
        let version = r402::proto::helpers::detect_version_bytes(payload_bytes)
            .map_err(|e| -> SchemeError { e.to_string().into() })?;
        let payload_value: serde_json::Value = serde_json::from_slice(payload_bytes)
            .map_err(|e| -> SchemeError { e.to_string().into() })?;
        let requirements_value: serde_json::Value = serde_json::from_slice(requirements_bytes)
            .map_err(|e| -> SchemeError { e.to_string().into() })?;

        self.verify_http(version, payload_value, requirements_value)
            .await
    }

    /// Settles a payment from raw JSON bytes.
    ///
    /// Operates at the network boundary — auto-detects protocol version.
    ///
    /// # Errors
    ///
    /// Returns an error on network failure, non-200 response, or parse error.
    pub async fn settle_from_bytes(
        &self,
        payload_bytes: &[u8],
        requirements_bytes: &[u8],
    ) -> Result<SettleResponse, SchemeError> {
        let version = r402::proto::helpers::detect_version_bytes(payload_bytes)
            .map_err(|e| -> SchemeError { e.to_string().into() })?;
        let payload_value: serde_json::Value = serde_json::from_slice(payload_bytes)
            .map_err(|e| -> SchemeError { e.to_string().into() })?;
        let requirements_value: serde_json::Value = serde_json::from_slice(requirements_bytes)
            .map_err(|e| -> SchemeError { e.to_string().into() })?;

        self.settle_http(version, payload_value, requirements_value)
            .await
    }
}

impl std::fmt::Debug for HttpFacilitatorClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpFacilitatorClient")
            .field("url", &self.url)
            .field("identifier", &self.identifier)
            .field("has_auth_provider", &self.auth_provider.is_some())
            .finish_non_exhaustive()
    }
}

impl FacilitatorClient for HttpFacilitatorClient {
    fn verify<'a>(
        &'a self,
        payload: &'a PaymentPayload,
        requirements: &'a PaymentRequirements,
    ) -> BoxFuture<'a, Result<VerifyResponse, SchemeError>> {
        Box::pin(async move {
            let payload_value = serde_json::to_value(payload)
                .map_err(|e| -> SchemeError { format!("Serialize payload: {e}").into() })?;
            let requirements_value = serde_json::to_value(requirements)
                .map_err(|e| -> SchemeError { format!("Serialize requirements: {e}").into() })?;

            self.verify_http(2, payload_value, requirements_value).await
        })
    }

    fn settle<'a>(
        &'a self,
        payload: &'a PaymentPayload,
        requirements: &'a PaymentRequirements,
    ) -> BoxFuture<'a, Result<SettleResponse, SchemeError>> {
        Box::pin(async move {
            let payload_value = serde_json::to_value(payload)
                .map_err(|e| -> SchemeError { format!("Serialize payload: {e}").into() })?;
            let requirements_value = serde_json::to_value(requirements)
                .map_err(|e| -> SchemeError { format!("Serialize requirements: {e}").into() })?;

            self.settle_http(2, payload_value, requirements_value).await
        })
    }

    fn get_supported(&self) -> BoxFuture<'_, Result<SupportedResponse, SchemeError>> {
        Box::pin(async move {
            let response = self
                .client
                .get(format!("{}/supported", self.url))
                .headers(self.supported_headers())
                .send()
                .await
                .map_err(|e| -> SchemeError {
                    format!("Facilitator get_supported request failed: {e}").into()
                })?;

            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                return Err(format!("Facilitator get_supported failed ({status}): {text}").into());
            }

            let result: SupportedResponse = response.json().await.map_err(|e| -> SchemeError {
                format!("Facilitator get_supported response parse error: {e}").into()
            })?;

            Ok(result)
        })
    }
}
