//! Remote facilitator client for the Casper exact scheme.
//!
//! Casper settlement is executed by a hosted (or self-hosted) facilitator
//! that holds a funded account and a node connection. This crate talks to it
//! over `/verify`, `/settle`, and `/supported`.

mod config;
mod transport;
pub mod verify;

use std::future::Future;
use std::time::Duration;

pub use config::{
    CASPER_DOCS_URL, CasperFacilitatorConfig, CasperFacilitatorConfigError,
    DEFAULT_FACILITATOR_URL, FACILITATOR_URL_ENV,
};
use r402_facilitator::Facilitator;
use r402_protocol::error::{FacilitatorError, FacilitatorTransportKind, VerificationError};
use r402_protocol::payment::{
    SettleRequest, SettleResponse, SupportedResponse, VerifyRequest, VerifyResponse,
};
pub use transport::ReqwestTransport;
pub use verify::{ValidatedPayment, validate_at, validate_request, validate_timing};

use crate::exact::payload::v2;

/// Facilitator for Casper exact scheme payments backed by a remote x402
/// facilitator service.
#[derive(Debug, Clone)]
pub struct CasperExactFacilitator<T> {
    config: CasperFacilitatorConfig,
    transport: T,
}

/// Transport abstraction over the facilitator's HTTP surface.
///
/// Callers MUST honour `timeout` from [`CasperFacilitatorConfig::timeout`].
pub trait FacilitatorTransport: Send + Sync {
    /// Sends a `POST` with a JSON body and returns the JSON response.
    fn post_json(
        &self,
        url: &url::Url,
        body: serde_json::Value,
        timeout: Duration,
    ) -> impl Future<Output = Result<serde_json::Value, FacilitatorError>> + Send;

    /// Sends a `GET` and returns the JSON response.
    fn get_json(
        &self,
        url: &url::Url,
        timeout: Duration,
    ) -> impl Future<Output = Result<serde_json::Value, FacilitatorError>> + Send;
}

impl<T> CasperExactFacilitator<T> {
    /// Creates a facilitator with an explicit configuration and transport.
    pub const fn new(config: CasperFacilitatorConfig, transport: T) -> Self {
        Self { config, transport }
    }

    /// Constructs a facilitator from a transport using hosted defaults.
    ///
    /// Currently infallible. [`Result`] so `try_new(transport)?` compiles.
    ///
    /// # Errors
    ///
    /// Currently never.
    #[allow(
        clippy::unnecessary_wraps,
        reason = "Result so try_new(provider)? compiles"
    )]
    pub fn try_new(transport: T) -> Result<Self, FacilitatorError> {
        Ok(Self::hosted(transport))
    }

    /// Creates a facilitator pointing at the hosted Casper facilitator.
    pub fn hosted(transport: T) -> Self {
        Self::new(CasperFacilitatorConfig::default(), transport)
    }

    /// Creates a facilitator configured from the environment.
    ///
    /// # Errors
    ///
    /// Returns [`CasperFacilitatorConfigError`] when
    /// [`FACILITATOR_URL_ENV`] is set to an invalid URL.
    pub fn from_env(transport: T) -> Result<Self, CasperFacilitatorConfigError> {
        Ok(Self::new(CasperFacilitatorConfig::from_env()?, transport))
    }

    /// Returns the active configuration.
    #[must_use]
    pub const fn config(&self) -> &CasperFacilitatorConfig {
        &self.config
    }

    /// Runs the local pre-flight validation pass, if enabled.
    fn preflight(&self, request: &serde_json::Value) -> Result<(), VerificationError> {
        if !self.config.validate_locally {
            return Ok(());
        }
        let typed = v2::VerifyRequest::from_verify(VerifyRequest::from(request.clone()))?;
        validate_request(&typed).map_err(VerificationError::from)?;
        Ok(())
    }
}

impl CasperExactFacilitator<ReqwestTransport> {
    /// Hosted facilitator with the default `reqwest` transport.
    #[must_use]
    pub fn hosted_http() -> Self {
        Self::hosted(ReqwestTransport::new())
    }

    /// Environment-configured facilitator with the default `reqwest` transport.
    ///
    /// # Errors
    ///
    /// Returns [`CasperFacilitatorConfigError`] when
    /// [`FACILITATOR_URL_ENV`] is set to an invalid URL.
    pub fn from_env_http() -> Result<Self, CasperFacilitatorConfigError> {
        Self::from_env(ReqwestTransport::new())
    }
}

fn decode_response<T: serde::de::DeserializeOwned>(
    response: serde_json::Value,
) -> Result<T, FacilitatorError> {
    serde_json::from_value(response)
        .map_err(|_| FacilitatorError::transport(FacilitatorTransportKind::MalformedSuccessBody))
}

impl<T> Facilitator for CasperExactFacilitator<T>
where
    T: FacilitatorTransport,
{
    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_casper::exact::verify", skip_all)
    )]
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        let body = request.into_json();
        self.preflight(&body)?;
        let response = self
            .transport
            .post_json(&self.config.verify_url(), body, self.config.timeout)
            .await?;
        decode_response(response)
    }

    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_casper::exact::settle", skip_all)
    )]
    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        let body = request.into_json();
        self.preflight(&body)?;
        let response = self
            .transport
            .post_json(&self.config.settle_url(), body, self.config.timeout)
            .await?;
        decode_response(response)
    }

    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_casper::exact::supported", skip_all)
    )]
    async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
        let response = self
            .transport
            .get_json(&self.config.supported_url(), self.config.timeout)
            .await?;
        decode_response(response)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use r402_protocol::UnixTimestamp;
    use r402_protocol::error::ErrorReason;

    use super::*;

    const PAYER: &str = "0076d080b4e769f0b29c77fc6472d6e425710840c2f46a4506e5544d2ce34f43a3";
    const PUBLIC_KEY: &str = "020376e4f8766e4f33bcc6e20b331b5163f363dc0106063b052ad38afe08637bd867";
    const PAYEE: &str = "00fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321";
    const ASSET: &str = "3d80df21ba4ee4d66a2a1f60c32570dd5685e4b279f6538162a5fd1314847c1e";

    #[derive(Debug)]
    struct MockTransport {
        response: serde_json::Value,
        calls: Mutex<Vec<(String, Duration)>>,
    }

    impl MockTransport {
        fn new(response: serde_json::Value) -> Self {
            Self {
                response,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn call_urls(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|(url, _)| url.clone())
                .collect()
        }

        fn call_timeouts(&self) -> Vec<Duration> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|(_, timeout)| *timeout)
                .collect()
        }
    }

    impl FacilitatorTransport for MockTransport {
        fn post_json(
            &self,
            url: &url::Url,
            _body: serde_json::Value,
            timeout: Duration,
        ) -> impl Future<Output = Result<serde_json::Value, FacilitatorError>> {
            self.calls.lock().unwrap().push((url.to_string(), timeout));
            std::future::ready(Ok(self.response.clone()))
        }

        fn get_json(
            &self,
            url: &url::Url,
            timeout: Duration,
        ) -> impl Future<Output = Result<serde_json::Value, FacilitatorError>> {
            self.calls.lock().unwrap().push((url.to_string(), timeout));
            std::future::ready(Ok(self.response.clone()))
        }
    }

    fn requirements(amount: &str) -> serde_json::Value {
        serde_json::json!({
            "scheme": "exact",
            "network": "casper:casper-test",
            "amount": amount,
            "payTo": PAYEE,
            "maxTimeoutSeconds": 300,
            "asset": ASSET,
            "extra": { "name": "Wrapped CSPR", "version": "1" }
        })
    }

    fn verify_request(amount: &str, valid_before: u64) -> VerifyRequest {
        let requirements = requirements(amount);
        VerifyRequest::from(serde_json::json!({
            "x402Version": 2,
            "paymentPayload": {
                "x402Version": 2,
                "accepted": requirements,
                "payload": {
                    "signature": format!("02{}", "aa".repeat(64)),
                    "publicKey": PUBLIC_KEY,
                    "authorization": {
                        "from": PAYER,
                        "to": PAYEE,
                        "value": amount,
                        "validAfter": "0",
                        "validBefore": valid_before.to_string(),
                        "nonce": "cc".repeat(32),
                    }
                }
            },
            "paymentRequirements": requirements
        }))
    }

    fn far_future() -> u64 {
        UnixTimestamp::now().as_secs() + 600
    }

    #[tokio::test]
    async fn verify_posts_to_the_configured_verify_endpoint() {
        let transport = MockTransport::new(serde_json::json!({
            "isValid": true,
            "payer": PAYER
        }));
        let facilitator = CasperExactFacilitator::hosted(transport);
        let response = facilitator
            .verify(verify_request("1500000000", far_future()))
            .await
            .unwrap();
        assert!(response.is_valid());
        assert_eq!(
            facilitator.transport.call_urls(),
            vec!["https://x402-facilitator.cspr.cloud/verify".to_owned()]
        );
        assert_eq!(
            facilitator.transport.call_timeouts(),
            vec![Duration::from_secs(30)]
        );
    }

    #[tokio::test]
    async fn settle_posts_to_the_configured_settle_endpoint() {
        let transport = MockTransport::new(serde_json::json!({
            "success": true,
            "payer": PAYER,
            "transaction": "9f".repeat(32),
            "network": "casper:casper-test",
            "amount": "1500000000"
        }));
        let facilitator = CasperExactFacilitator::hosted(transport);
        let request = SettleRequest::from(verify_request("1500000000", far_future()).into_json());
        let response = facilitator.settle(request).await.unwrap();
        assert!(response.is_success());
        assert_eq!(
            facilitator.transport.call_urls(),
            vec!["https://x402-facilitator.cspr.cloud/settle".to_owned()]
        );
    }

    #[tokio::test]
    async fn supported_gets_the_supported_endpoint() {
        let transport = MockTransport::new(serde_json::json!({
            "kinds": [{
                "x402Version": 2,
                "scheme": "exact",
                "network": "casper:casper-test",
                "extra": { "feePayer": PAYEE }
            }]
        }));
        let facilitator = CasperExactFacilitator::hosted(transport);
        let response = facilitator.supported().await.unwrap();
        assert_eq!(response.kinds.len(), 1);
        assert_eq!(response.kinds[0].network, "casper:casper-test");
        assert_eq!(
            facilitator.transport.call_urls(),
            vec!["https://x402-facilitator.cspr.cloud/supported".to_owned()]
        );
    }

    #[tokio::test]
    async fn preflight_rejects_expired_payments_without_a_round_trip() {
        let transport = MockTransport::new(serde_json::json!({ "isValid": true, "payer": PAYER }));
        let facilitator = CasperExactFacilitator::hosted(transport);
        let error = facilitator
            .verify(verify_request("1500000000", 1))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            FacilitatorError::Verification(VerificationError::Expired)
        ));
        assert!(
            facilitator.transport.call_urls().is_empty(),
            "an expired payment must not reach the network"
        );
    }

    #[tokio::test]
    async fn preflight_rejects_zero_amounts() {
        let transport = MockTransport::new(serde_json::json!({ "isValid": true, "payer": PAYER }));
        let facilitator = CasperExactFacilitator::hosted(transport);
        let error = facilitator
            .verify(verify_request("0", far_future()))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            FacilitatorError::Verification(VerificationError::InvalidPaymentAmount)
        ));
    }

    #[tokio::test]
    async fn preflight_can_be_disabled() {
        let transport = MockTransport::new(serde_json::json!({ "isValid": true, "payer": PAYER }));
        let facilitator = CasperExactFacilitator::new(
            CasperFacilitatorConfig::default().with_local_validation(false),
            transport,
        );
        let response = facilitator
            .verify(verify_request("1500000000", 1))
            .await
            .unwrap();
        assert!(response.is_valid());
        assert_eq!(facilitator.transport.call_urls().len(), 1);
    }

    #[tokio::test]
    async fn remote_invalid_response_is_surfaced_verbatim() {
        let transport = MockTransport::new(serde_json::json!({
            "isValid": false,
            "invalidReason": "insufficient_funds",
            "payer": PAYER
        }));
        let facilitator = CasperExactFacilitator::hosted(transport);
        let response = facilitator
            .verify(verify_request("1500000000", far_future()))
            .await
            .unwrap();
        assert!(matches!(
            response,
            VerifyResponse::Invalid {
                reason: ErrorReason::InsufficientFunds,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn malformed_remote_response_is_transport() {
        let transport = MockTransport::new(serde_json::json!({ "unexpected": true }));
        let facilitator = CasperExactFacilitator::hosted(transport);
        let error = facilitator
            .verify(verify_request("1500000000", far_future()))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            FacilitatorError::Transport {
                kind: FacilitatorTransportKind::MalformedSuccessBody
            }
        ));
    }

    #[tokio::test]
    async fn custom_base_url_is_honoured() {
        let transport = MockTransport::new(serde_json::json!({ "isValid": true, "payer": PAYER }));
        let facilitator = CasperExactFacilitator::new(
            CasperFacilitatorConfig::new("https://facilitator.internal/x402/").unwrap(),
            transport,
        );
        let _ = facilitator
            .verify(verify_request("1500000000", far_future()))
            .await
            .unwrap();
        assert_eq!(
            facilitator.transport.call_urls(),
            vec!["https://facilitator.internal/x402/verify".to_owned()]
        );
    }

    #[tokio::test]
    async fn configured_timeout_is_forwarded_to_the_transport() {
        let transport = MockTransport::new(serde_json::json!({ "isValid": true, "payer": PAYER }));
        let facilitator = CasperExactFacilitator::new(
            CasperFacilitatorConfig::default().with_timeout(Duration::from_secs(7)),
            transport,
        );
        let _ = facilitator
            .verify(verify_request("1500000000", far_future()))
            .await
            .unwrap();
        assert_eq!(
            facilitator.transport.call_timeouts(),
            vec![Duration::from_secs(7)]
        );
    }

    #[test]
    fn try_new_is_result() {
        let transport = MockTransport::new(serde_json::json!({ "isValid": true, "payer": PAYER }));
        let _ = CasperExactFacilitator::try_new(transport).expect("try_new is infallible");
    }
}
