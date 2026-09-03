//! Default HTTP transport for [`super::CasperExactFacilitator`].
//!
//! Status mapping matches [`r402_facilitator`] `FacilitatorClient`: non-2xx
//! Valid/Success is Transport (502), not paid; 2xx Success with an empty
//! `transaction` is `MalformedSuccessBody`.

use std::time::Duration;

use r402_protocol::error::{FacilitatorError, FacilitatorTransportKind};
use r402_protocol::payment::{
    Base64Bytes, Extensions, SettleResponse, SupportedResponse, VerifyResponse,
};

use super::FacilitatorTransport;

/// Raw HTTP result from [`FacilitatorTransport`].
///
/// Status and headers are required so verify/settle/supported can apply the
/// same 2xx vs 4xx rules as `FacilitatorClient` instead of treating any JSON
/// body as success.
#[derive(Debug, Clone)]
pub struct FacilitatorHttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response headers as `(name, value)` pairs.
    pub headers: Vec<(String, String)>,
    /// Raw body bytes.
    pub body: Vec<u8>,
}

impl FacilitatorHttpResponse {
    /// Builds a response with no headers.
    #[must_use]
    pub const fn new(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body,
        }
    }
}

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
    ) -> Result<FacilitatorHttpResponse, FacilitatorError> {
        let response = self
            .client
            .post(url.clone())
            .timeout(timeout)
            .json(&body)
            .send()
            .await
            .map_err(|e| map_reqwest(&e))?;
        read_response(response).await
    }

    async fn get_json(
        &self,
        url: &url::Url,
        timeout: Duration,
    ) -> Result<FacilitatorHttpResponse, FacilitatorError> {
        let response = self
            .client
            .get(url.clone())
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| map_reqwest(&e))?;
        read_response(response).await
    }
}

fn map_reqwest(err: &reqwest::Error) -> FacilitatorError {
    if err.is_timeout() {
        FacilitatorError::transport(FacilitatorTransportKind::Timeout)
    } else {
        FacilitatorError::transport(FacilitatorTransportKind::Io)
    }
}

async fn read_response(
    response: reqwest::Response,
) -> Result<FacilitatorHttpResponse, FacilitatorError> {
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned()))
        })
        .collect();
    let body = response
        .bytes()
        .await
        .map_err(|e| map_reqwest(&e))?
        .to_vec();
    Ok(FacilitatorHttpResponse {
        status,
        headers,
        body,
    })
}

const fn is_success(status: u16) -> bool {
    status >= 200 && status < 300
}

const fn status_transport(status: u16) -> FacilitatorError {
    FacilitatorError::transport(FacilitatorTransportKind::HttpStatus { status })
}

pub(super) fn parse_verify_body(
    raw: &FacilitatorHttpResponse,
) -> Result<VerifyResponse, FacilitatorError> {
    let parsed = match serde_json::from_slice::<VerifyResponse>(&raw.body) {
        Ok(parsed) => parsed,
        Err(_) if is_success(raw.status) => {
            return Err(FacilitatorError::transport(
                FacilitatorTransportKind::MalformedSuccessBody,
            ));
        }
        Err(_) => return Err(status_transport(raw.status)),
    };
    if is_success(raw.status) || !parsed.is_valid() {
        return Ok(attach_verify(parsed, raw));
    }
    Err(status_transport(raw.status))
}

pub(super) fn parse_settle_body(
    raw: &FacilitatorHttpResponse,
) -> Result<SettleResponse, FacilitatorError> {
    let parsed = match serde_json::from_slice::<SettleResponse>(&raw.body) {
        Ok(parsed) => parsed,
        Err(_) if is_success(raw.status) => {
            return Err(FacilitatorError::transport(
                FacilitatorTransportKind::MalformedSuccessBody,
            ));
        }
        Err(_) => return Err(status_transport(raw.status)),
    };
    if is_success(raw.status) {
        if settle_success_missing_transaction(&parsed) {
            return Err(FacilitatorError::transport(
                FacilitatorTransportKind::MalformedSuccessBody,
            ));
        }
        return Ok(attach_settle(parsed, raw));
    }
    if parsed.is_success() {
        return Err(status_transport(raw.status));
    }
    Ok(attach_settle(parsed, raw))
}

fn settle_success_missing_transaction(response: &SettleResponse) -> bool {
    match response {
        SettleResponse::Success { transaction, .. } => transaction.is_empty(),
        _ => false,
    }
}

pub(super) fn parse_supported_body(
    raw: &FacilitatorHttpResponse,
) -> Result<SupportedResponse, FacilitatorError> {
    if is_success(raw.status) {
        serde_json::from_slice(&raw.body).map_err(|_| {
            FacilitatorError::transport(FacilitatorTransportKind::MalformedSuccessBody)
        })
    } else {
        Err(status_transport(raw.status))
    }
}

fn attach_verify(mut response: VerifyResponse, raw: &FacilitatorHttpResponse) -> VerifyResponse {
    response.set_extension_responses(parse_extension_responses(&raw.headers));
    response
}

fn attach_settle(mut response: SettleResponse, raw: &FacilitatorHttpResponse) -> SettleResponse {
    response.set_extension_responses(parse_extension_responses(&raw.headers));
    response
}

fn parse_extension_responses(headers: &[(String, String)]) -> Extensions {
    let Some((_, raw)) = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("extension-responses"))
    else {
        return Extensions::new();
    };
    decode_extension_responses(raw).unwrap_or_default()
}

fn decode_extension_responses(header: &str) -> Option<Extensions> {
    let bytes = Base64Bytes(header.as_bytes().to_vec());
    let decoded = bytes.decode().ok()?;
    let value: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    if !value.is_object() {
        return None;
    }
    serde_json::from_value(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json_response(status: u16, value: &serde_json::Value) -> FacilitatorHttpResponse {
        FacilitatorHttpResponse::new(status, serde_json::to_vec(value).unwrap())
    }

    #[test]
    fn default_builds() {
        let transport = ReqwestTransport::default();
        assert!(!format!("{:?}", transport.client()).is_empty());
    }

    #[test]
    fn verify_2xx_malformed_is_transport() {
        let err = parse_verify_body(&FacilitatorHttpResponse::new(200, b"not-json".to_vec()))
            .unwrap_err();
        assert!(matches!(
            err,
            FacilitatorError::Transport {
                kind: FacilitatorTransportKind::MalformedSuccessBody
            }
        ));
    }

    #[test]
    fn verify_5xx_malformed_is_http_status() {
        let err =
            parse_verify_body(&FacilitatorHttpResponse::new(502, b"oops".to_vec())).unwrap_err();
        assert!(matches!(
            err,
            FacilitatorError::Transport {
                kind: FacilitatorTransportKind::HttpStatus { status: 502 }
            }
        ));
    }

    #[test]
    fn verify_4xx_valid_json_is_transport_not_paid() {
        let err = parse_verify_body(&json_response(
            400,
            &serde_json::json!({ "isValid": true, "payer": "00aa" }),
        ))
        .unwrap_err();
        assert!(matches!(
            err,
            FacilitatorError::Transport {
                kind: FacilitatorTransportKind::HttpStatus { status: 400 }
            }
        ));
        assert!(
            err.as_payment_problem().is_none(),
            "non-2xx Valid must be 502, not 402"
        );
    }

    #[test]
    fn verify_4xx_invalid_json_is_ok() {
        let response = parse_verify_body(&json_response(
            400,
            &serde_json::json!({
                "isValid": false,
                "invalidReason": "insufficient_funds",
                "payer": "00aa"
            }),
        ))
        .unwrap();
        assert!(!response.is_valid());
    }

    #[test]
    fn settle_2xx_missing_transaction_is_malformed() {
        let err = parse_settle_body(&json_response(
            200,
            &serde_json::json!({
                "success": true,
                "payer": "00aa",
                "network": "casper:casper-test"
            }),
        ))
        .unwrap_err();
        assert!(matches!(
            err,
            FacilitatorError::Transport {
                kind: FacilitatorTransportKind::MalformedSuccessBody
            }
        ));
    }

    #[test]
    fn settle_4xx_success_json_is_transport() {
        let err = parse_settle_body(&json_response(
            400,
            &serde_json::json!({
                "success": true,
                "payer": "00aa",
                "transaction": "9f".repeat(32),
                "network": "casper:casper-test"
            }),
        ))
        .unwrap_err();
        assert!(matches!(
            err,
            FacilitatorError::Transport {
                kind: FacilitatorTransportKind::HttpStatus { status: 400 }
            }
        ));
        assert!(err.as_payment_problem().is_none());
    }

    #[test]
    fn settle_4xx_failure_json_is_ok() {
        let response = parse_settle_body(&json_response(
            400,
            &serde_json::json!({
                "success": false,
                "errorReason": "insufficient_funds",
                "transaction": "",
                "network": "casper:casper-test"
            }),
        ))
        .unwrap();
        assert!(!response.is_success());
    }

    #[test]
    fn supported_non_2xx_is_http_status() {
        let err = parse_supported_body(&json_response(503, &serde_json::json!({}))).unwrap_err();
        assert!(matches!(
            err,
            FacilitatorError::Transport {
                kind: FacilitatorTransportKind::HttpStatus { status: 503 }
            }
        ));
    }

    #[test]
    fn verify_attaches_extension_responses() {
        let encoded = Base64Bytes::encode(br#"{"demo":{}}"#).to_string();
        let mut raw = json_response(
            200,
            &serde_json::json!({ "isValid": true, "payer": "00aa" }),
        );
        raw.headers
            .push(("EXTENSION-RESPONSES".to_owned(), encoded));
        let response = parse_verify_body(&raw).unwrap();
        assert!(response.is_valid());
        assert!(response.extension_responses().get("demo").is_some());
    }
}
