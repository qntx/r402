//! Default HTTP transport for [`super::CasperExactFacilitator`].

use std::time::Duration;

use r402_protocol::error::{FacilitatorError, FacilitatorTransportKind};
use reqwest::StatusCode;

use super::FacilitatorTransport;

/// `reqwest`-backed implementation of [`FacilitatorTransport`].
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

    /// Builds a transport that reuses an existing `reqwest::Client`.
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
            .map_err(|e| map_reqwest(&e))?;
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
            .map_err(|e| map_reqwest(&e))?;
        read_json(response).await
    }
}

fn map_reqwest(err: &reqwest::Error) -> FacilitatorError {
    if err.is_timeout() {
        FacilitatorError::transport(FacilitatorTransportKind::Timeout)
    } else {
        FacilitatorError::transport(FacilitatorTransportKind::Io)
    }
}

async fn read_json(response: reqwest::Response) -> Result<serde_json::Value, FacilitatorError> {
    let status = response.status();
    let body = response.bytes().await.map_err(|e| map_reqwest(&e))?;
    parse_json_body(status, &body)
}

fn parse_json_body(status: StatusCode, body: &[u8]) -> Result<serde_json::Value, FacilitatorError> {
    match serde_json::from_slice::<serde_json::Value>(body) {
        // Well-formed JSON on 4xx/5xx is returned so Invalid verify
        // responses stay 402 rather than 502.
        Ok(value) => Ok(value),
        Err(_) if status.is_success() => Err(FacilitatorError::transport(
            FacilitatorTransportKind::MalformedSuccessBody,
        )),
        Err(_) => Err(FacilitatorError::transport(
            FacilitatorTransportKind::HttpStatus {
                status: status.as_u16(),
            },
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_builds() {
        let transport = ReqwestTransport::default();
        assert!(!format!("{:?}", transport.client()).is_empty());
    }

    #[test]
    fn success_malformed_body_is_transport() {
        let err = parse_json_body(StatusCode::OK, b"not-json").unwrap_err();
        assert!(matches!(
            err,
            FacilitatorError::Transport {
                kind: FacilitatorTransportKind::MalformedSuccessBody
            }
        ));
    }

    #[test]
    fn error_malformed_body_is_http_status() {
        let err = parse_json_body(StatusCode::BAD_GATEWAY, b"oops").unwrap_err();
        assert!(matches!(
            err,
            FacilitatorError::Transport {
                kind: FacilitatorTransportKind::HttpStatus { status: 502 }
            }
        ));
    }

    #[test]
    fn success_json_is_ok() {
        let value = parse_json_body(StatusCode::OK, br#"{"isValid":true}"#).unwrap();
        assert_eq!(value["isValid"], true);
    }
}
