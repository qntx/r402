//! Facilitator-side verification and settlement for the Casper exact scheme.
//!
//! Casper settlement is executed by a facilitator that holds a funded Casper
//! account and a node connection: it relays the buyer's signed EIP-712
//! authorisation to the CEP-18 token's `transfer_with_authorization` entry
//! point and pays the gas. The Casper Association operates a hosted instance
//! at <https://x402-facilitator.cspr.cloud> (documented at
//! <https://docs.cspr.cloud>).
//!
//! [`CasperExactFacilitator`] implements [`Facilitator`] against such a
//! service, so a Casper chain plugs into the same registry, hook pipeline,
//! and Axum middleware as the EVM and SVM crates. Every request is validated
//! locally first (see [`crate::exact::verify`]) so a malformed payment is
//! rejected with a precise typed error instead of an opaque remote failure.

mod config;
#[cfg(feature = "http-client")]
mod transport;

use std::time::Duration;

pub use config::{
    CasperFacilitatorConfig, CasperFacilitatorConfigError, DEFAULT_FACILITATOR_URL,
    FACILITATOR_URL_ENV,
};
use r402_core::error::VerificationError;
use r402_core::facilitator::{Facilitator, FacilitatorError};
use r402_core::wire;
#[cfg(feature = "http-client")]
pub use transport::ReqwestTransport;

use crate::exact::types::v2;
use crate::exact::verify::validate_request;

/// Facilitator for Casper exact scheme payments backed by a remote x402
/// facilitator service.
#[derive(Debug, Clone)]
pub struct CasperExactFacilitator<T> {
    config: CasperFacilitatorConfig,
    transport: T,
}

/// Transport abstraction over the facilitator's HTTP surface.
///
/// Implemented for any HTTP stack the host application already uses; r402
/// therefore does not force a specific client (or TLS backend) on downstream
/// consumers of this crate.
///
/// Callers MUST honour `timeout` (from [`CasperFacilitatorConfig::timeout`])
/// so that operator configuration is not silently ignored.
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
    ///
    /// # Errors
    ///
    /// Returns the typed [`VerificationError`] for the first failed check.
    fn preflight(&self, request: &serde_json::Value) -> Result<(), VerificationError> {
        if !self.config.validate_locally {
            return Ok(());
        }
        let typed = v2::VerifyRequest::from_verify(wire::VerifyRequest::from(request.clone()))?;
        validate_request(&typed).map_err(VerificationError::from)?;
        Ok(())
    }
}

#[cfg(feature = "http-client")]
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

impl<T> Facilitator for CasperExactFacilitator<T>
where
    T: FacilitatorTransport,
{
    async fn verify(
        &self,
        request: wire::VerifyRequest,
    ) -> Result<wire::VerifyResponse, FacilitatorError> {
        let body = request.into_json();
        self.preflight(&body)?;
        let response = self
            .transport
            .post_json(&self.config.verify_url(), body, self.config.timeout)
            .await?;
        serde_json::from_value(response)
            .map_err(|e| FacilitatorError::from(VerificationError::InvalidFormat(e.to_string())))
    }

    async fn settle(
        &self,
        request: wire::SettleRequest,
    ) -> Result<wire::SettleResponse, FacilitatorError> {
        let body = request.into_json();
        self.preflight(&body)?;
        let response = self
            .transport
            .post_json(&self.config.settle_url(), body, self.config.timeout)
            .await?;
        serde_json::from_value(response)
            .map_err(|e| FacilitatorError::from(VerificationError::InvalidFormat(e.to_string())))
    }

    async fn supported(&self) -> Result<wire::SupportedResponse, FacilitatorError> {
        let response = self
            .transport
            .get_json(&self.config.supported_url(), self.config.timeout)
            .await?;
        serde_json::from_value(response)
            .map_err(|e| FacilitatorError::from(VerificationError::InvalidFormat(e.to_string())))
    }
}

#[allow(
    clippy::indexing_slicing,
    reason = "tests index serde_json values; panic-on-missing-key is the desired assertion behaviour"
)]
#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use r402_core::error_reason::ErrorReason;

    use super::*;

    /// Secp256k1 fixture from `scheme_exact_casper.md` (publicKey → from).
    const PAYER: &str = "0076d080b4e769f0b29c77fc6472d6e425710840c2f46a4506e5544d2ce34f43a3";
    const PUBLIC_KEY: &str = "020376e4f8766e4f33bcc6e20b331b5163f363dc0106063b052ad38afe08637bd867";
    const PAYEE: &str = "00fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321";
    const ASSET: &str = "3d80df21ba4ee4d66a2a1f60c32570dd5685e4b279f6538162a5fd1314847c1e";

    /// Records the URLs + timeouts it was called with and replays a canned response.
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

    fn verify_request(amount: &str, valid_before: u64) -> wire::VerifyRequest {
        let requirements = requirements(amount);
        wire::VerifyRequest::from(serde_json::json!({
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
        crate::exact::verify::now_unix() + 600
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
        let request =
            wire::SettleRequest::from(verify_request("1500000000", far_future()).into_json());
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
            wire::VerifyResponse::Invalid {
                reason: ErrorReason::InsufficientFunds,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn malformed_remote_response_becomes_an_invalid_format_error() {
        let transport = MockTransport::new(serde_json::json!({ "unexpected": true }));
        let facilitator = CasperExactFacilitator::hosted(transport);
        let error = facilitator
            .verify(verify_request("1500000000", far_future()))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            FacilitatorError::Verification(VerificationError::InvalidFormat(_))
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
}
