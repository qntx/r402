//! Default HTTP transport for [`super::CasperExactFacilitator`].
//!
//! Gated behind the `http-client` feature so applications that already
//! own a client (or use a different TLS backend) are not forced to pull
//! `reqwest`.

use std::time::Duration;

use r402_core::error::VerificationError;
use r402_core::facilitator::FacilitatorError;

use super::FacilitatorTransport;

/// `reqwest`-backed implementation of [`FacilitatorTransport`].
///
/// Honour the per-call `timeout` from
/// [`super::CasperFacilitatorConfig`] so operator configuration is not
/// ignored.
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl Default for ReqwestTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl ReqwestTransport {
    /// Builds a transport with a fresh default `reqwest::Client`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    /// Builds a transport that reuses an existing `reqwest::Client`
    /// (shared connection pool, custom TLS, proxies, …).
    #[must_use]
    pub const fn with_client(client: reqwest::Client) -> Self {
        Self { client }
    }

    /// Returns a reference to the underlying HTTP client.
    #[must_use]
    pub const fn client(&self) -> &reqwest::Client {
        &self.client
    }
}

impl FacilitatorTransport for ReqwestTransport {
    async fn post_json(
        &self,
        url: &url::Url,
        body: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, FacilitatorError> {
        let response = self
            .client
            .post(url.clone())
            .timeout(timeout)
            .json(&body)
            .send()
            .await
            .map_err(FacilitatorError::internal)?;
        read_json(response).await
    }

    async fn get_json(
        &self,
        url: &url::Url,
        timeout: Duration,
    ) -> Result<serde_json::Value, FacilitatorError> {
        let response = self
            .client
            .get(url.clone())
            .timeout(timeout)
            .send()
            .await
            .map_err(FacilitatorError::internal)?;
        read_json(response).await
    }
}

async fn read_json(response: reqwest::Response) -> Result<serde_json::Value, FacilitatorError> {
    let status = response.status();
    let body = response.text().await.map_err(FacilitatorError::internal)?;
    if !status.is_success() {
        return Err(FacilitatorError::from(VerificationError::InvalidFormat(
            format!("facilitator HTTP {status}: {body}"),
        )));
    }
    serde_json::from_str(&body)
        .map_err(|e| FacilitatorError::from(VerificationError::InvalidFormat(e.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_builds() {
        let transport = ReqwestTransport::default();
        // Client is constructed and addressable.
        assert!(!format!("{:?}", transport.client()).is_empty());
    }
}
