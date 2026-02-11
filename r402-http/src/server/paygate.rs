//! Core payment gate logic for enforcing x402 payments.
//!
//! The [`Paygate`] struct handles the full payment lifecycle:
//! extracting headers, verifying with the facilitator, settling on-chain,
//! and returning 402 responses when payment is required.
//!
//! Protocol-specific behavior is provided by [`PaygateProtocol`] (see
//! [`super::protocol`]), and pricing strategies by [`PriceTagSource`]
//! (see [`super::price_source`]).

use axum_core::extract::Request;
use axum_core::response::{IntoResponse, Response};
use http::{HeaderMap, HeaderValue};
use r402::facilitator::Facilitator;
use r402::proto;
use r402::proto::v2;
use std::convert::Infallible;
use std::sync::Arc;
use tower::Service;
use url::Url;

use r402::proto::Base64Bytes;
#[cfg(feature = "telemetry")]
use tracing::Instrument;
#[cfg(feature = "telemetry")]
use tracing::instrument;

use r402::hooks::{FailureRecovery, HookDecision};

use super::error::{PaygateError, VerificationError};
use super::hooks::{PaygateHooks, SettleContext, VerifyContext};
use super::protocol::PaygateProtocol;

/// Builder for resource information that can be used with both V1 and V2 protocols.
#[derive(Debug, Clone)]
pub struct ResourceInfoBuilder {
    /// Description of the protected resource
    pub description: String,
    /// MIME type of the protected resource
    pub mime_type: String,
    /// Optional explicit URL of the protected resource
    pub url: Option<String>,
}

impl Default for ResourceInfoBuilder {
    fn default() -> Self {
        Self {
            description: String::new(),
            mime_type: "application/json".to_string(),
            url: None,
        }
    }
}

impl ResourceInfoBuilder {
    /// Determines the resource URL (static or dynamic).
    ///
    /// If `url` is set, returns it directly. Otherwise, constructs a URL by combining
    /// the base URL with the request URI's path and query.
    ///
    /// # Panics
    ///
    /// Panics if internal URL construction fails (should not happen in practice).
    #[allow(clippy::unwrap_used)]
    pub fn as_resource_info(&self, base_url: Option<&Url>, req: &Request) -> v2::ResourceInfo {
        let url = self.url.clone().unwrap_or_else(|| {
            let mut url = base_url.cloned().unwrap_or_else(|| {
                let host = req.headers().get("host").and_then(|h| h.to_str().ok()).unwrap_or("localhost");
                let origin = format!("http://{host}");
                let url = Url::parse(&origin).unwrap_or_else(|_| Url::parse("http://localhost").unwrap());
                #[cfg(feature = "telemetry")]
                tracing::warn!(
                    "X402Middleware base_url is not configured; using {url} as origin for resource resolution"
                );
                url
            });
            let request_uri = req.uri();
            url.set_path(request_uri.path());
            url.set_query(request_uri.query());
            url.to_string()
        });
        v2::ResourceInfo {
            description: self.description.clone(),
            mime_type: self.mime_type.clone(),
            url,
        }
    }
}

/// Unified payment gate that works with both V1 and V2 protocols.
///
/// The protocol version is determined by the price tag type parameter `P`, which must
/// implement [`PaygateProtocol`]. Use `V1PriceTag` for V1 protocol or `V2PriceTag`
/// (alias for `v2::PaymentRequirements`) for V2 protocol.
#[allow(missing_debug_implementations)] // generic types may not implement Debug
pub struct Paygate<TPriceTag, TFacilitator> {
    /// The facilitator for verifying and settling payments
    pub facilitator: TFacilitator,
    /// Whether to settle before or after request execution
    pub settle_before_execution: bool,
    /// Accepted payment requirements
    pub accepts: Arc<Vec<TPriceTag>>,
    /// Resource information for the protected endpoint
    pub resource: v2::ResourceInfo,
    /// Lifecycle hooks for verify/settle operations.
    ///
    /// Hooks are executed in order; the first abort or recovery wins.
    pub hooks: Arc<[Arc<dyn PaygateHooks>]>,
}

impl<TPriceTag, TFacilitator> Paygate<TPriceTag, TFacilitator> {
    /// Calls the inner service with proper telemetry instrumentation.
    async fn call_inner<
        ReqBody,
        ResBody,
        S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>>,
    >(
        mut inner: S,
        req: http::Request<ReqBody>,
    ) -> Result<http::Response<ResBody>, S::Error>
    where
        S::Future: Send,
    {
        #[cfg(feature = "telemetry")]
        {
            inner
                .call(req)
                .instrument(tracing::info_span!("inner"))
                .await
        }
        #[cfg(not(feature = "telemetry"))]
        {
            inner.call(req).await
        }
    }
}

impl<TPriceTag, TFacilitator> Paygate<TPriceTag, TFacilitator>
where
    TPriceTag: PaygateProtocol,
    TFacilitator: Facilitator + Sync,
{
    /// Handles an incoming request, processing payment if required.
    ///
    /// Returns 402 response if payment fails.
    /// Otherwise, returns the response from the inner service.
    ///
    /// # Errors
    ///
    /// This method is infallible (`Infallible` error type).
    #[cfg_attr(
        feature = "telemetry",
        instrument(name = "x402.handle_request", skip_all)
    )]
    pub async fn handle_request<
        ReqBody,
        ResBody,
        S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>>,
    >(
        self,
        inner: S,
        req: http::Request<ReqBody>,
    ) -> Result<Response, Infallible>
    where
        S::Response: IntoResponse,
        S::Error: IntoResponse,
        S::Future: Send,
    {
        match self.handle_request_fallible(inner, req).await {
            Ok(response) => Ok(response),
            Err(err) => Ok(TPriceTag::error_into_response(
                err,
                &self.accepts,
                &self.resource,
            )),
        }
    }

    /// Gets enriched price tags with facilitator capabilities.
    pub async fn enrich_accepts(&mut self) {
        let capabilities = self.facilitator.supported().await.unwrap_or_default();

        let accepts = (*self.accepts)
            .clone()
            .into_iter()
            .map(|mut pt| {
                pt.enrich_with_capabilities(&capabilities);
                pt
            })
            .collect::<Vec<_>>();
        self.accepts = Arc::new(accepts);
    }

    /// Handles an incoming request, returning errors as `PaygateError`.
    ///
    /// This is the fallible version of `handle_request` that returns an actual error
    /// instead of turning it into 402 Payment Required response.
    ///
    /// # Errors
    ///
    /// Returns [`PaygateError`] if payment processing fails.
    pub async fn handle_request_fallible<
        ReqBody,
        ResBody,
        S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>>,
    >(
        &self,
        inner: S,
        req: http::Request<ReqBody>,
    ) -> Result<Response, PaygateError>
    where
        S::Response: IntoResponse,
        S::Error: IntoResponse,
        S::Future: Send,
    {
        let header = extract_payment_header(req.headers(), TPriceTag::PAYMENT_HEADER_NAME).ok_or(
            VerificationError::PaymentHeaderRequired(TPriceTag::PAYMENT_HEADER_NAME),
        )?;
        let payment_payload = extract_payment_payload::<TPriceTag::PaymentPayload>(header)
            .ok_or(VerificationError::InvalidPaymentHeader)?;

        let verify_request =
            TPriceTag::make_verify_request(payment_payload, &self.accepts, &self.resource)?;

        if self.settle_before_execution {
            #[cfg(feature = "telemetry")]
            tracing::debug!("Settling payment before request execution");

            let settlement = self.settle_payment(verify_request.into()).await?;

            if let proto::SettleResponse::Error {
                reason, message, ..
            } = &settlement
            {
                let detail = message.as_deref().unwrap_or(reason.as_str());
                return Err(PaygateError::Settlement(detail.to_owned()));
            }

            let header_value = settlement_to_header(settlement)?;

            let response = match Self::call_inner(inner, req).await {
                Ok(response) => response,
                Err(err) => return Ok(err.into_response()),
            };

            let mut res = response;
            res.headers_mut().insert("X-Payment-Response", header_value);
            Ok(res.into_response())
        } else {
            #[cfg(feature = "telemetry")]
            tracing::debug!("Settling payment after request execution");

            let verify_response = self.verify_payment(verify_request.clone()).await?;

            TPriceTag::validate_verify_response(verify_response)?;

            let response = match Self::call_inner(inner, req).await {
                Ok(response) => response,
                Err(err) => return Ok(err.into_response()),
            };

            if response.status().is_client_error() || response.status().is_server_error() {
                return Ok(response.into_response());
            }

            let settlement = self.settle_payment(verify_request.into()).await?;

            if let proto::SettleResponse::Error {
                reason, message, ..
            } = &settlement
            {
                let detail = message.as_deref().unwrap_or(reason.as_str());
                return Err(PaygateError::Settlement(detail.to_owned()));
            }

            let header_value = settlement_to_header(settlement)?;

            let mut res = response;
            res.headers_mut().insert("X-Payment-Response", header_value);
            Ok(res.into_response())
        }
    }

    /// Verifies a payment with the facilitator, executing lifecycle hooks.
    ///
    /// # Errors
    ///
    /// Returns [`VerificationError`] if verification fails or a before-hook aborts.
    pub async fn verify_payment(
        &self,
        verify_request: proto::VerifyRequest,
    ) -> Result<proto::VerifyResponse, VerificationError> {
        let ctx = VerifyContext {
            request: verify_request.clone(),
        };

        for hook in self.hooks.iter() {
            if let HookDecision::Abort { reason, .. } = hook.before_verify(&ctx).await {
                return Err(VerificationError::VerificationFailed(reason));
            }
        }

        match self
            .facilitator
            .verify(verify_request)
            .await
            .map_err(|e| VerificationError::VerificationFailed(format!("{e}")))
        {
            Ok(response) => {
                for hook in self.hooks.iter() {
                    hook.after_verify(&ctx, &response).await;
                }
                Ok(response)
            }
            Err(err) => {
                let err_msg = err.to_string();
                for hook in self.hooks.iter() {
                    if let FailureRecovery::Recovered(response) =
                        hook.on_verify_failure(&ctx, &err_msg).await
                    {
                        return Ok(response);
                    }
                }
                Err(err)
            }
        }
    }

    /// Settles a payment with the facilitator, executing lifecycle hooks.
    ///
    /// # Errors
    ///
    /// Returns [`PaygateError`] if settlement fails or a before-hook aborts.
    pub async fn settle_payment(
        &self,
        settle_request: proto::SettleRequest,
    ) -> Result<proto::SettleResponse, PaygateError> {
        let ctx = SettleContext {
            request: settle_request.clone(),
        };

        for hook in self.hooks.iter() {
            if let HookDecision::Abort { reason, .. } = hook.before_settle(&ctx).await {
                return Err(PaygateError::Settlement(reason));
            }
        }

        match self
            .facilitator
            .settle(settle_request)
            .await
            .map_err(|e| PaygateError::Settlement(format!("{e}")))
        {
            Ok(response) => {
                for hook in self.hooks.iter() {
                    hook.after_settle(&ctx, &response).await;
                }
                Ok(response)
            }
            Err(err) => {
                let err_msg = err.to_string();
                for hook in self.hooks.iter() {
                    if let FailureRecovery::Recovered(response) =
                        hook.on_settle_failure(&ctx, &err_msg).await
                    {
                        return Ok(response);
                    }
                }
                Err(err)
            }
        }
    }
}

/// Extracts the payment header value from the header map.
fn extract_payment_header<'a>(header_map: &'a HeaderMap, header_name: &'a str) -> Option<&'a [u8]> {
    header_map.get(header_name).map(HeaderValue::as_bytes)
}

/// Extracts and deserializes the payment payload from base64-encoded header bytes.
fn extract_payment_payload<T>(header_bytes: &[u8]) -> Option<T>
where
    T: serde::de::DeserializeOwned,
{
    let base64 = Base64Bytes::from(header_bytes).decode().ok()?;
    let value = serde_json::from_slice(base64.as_ref()).ok()?;
    Some(value)
}

/// Converts a [`proto::SettleResponse`] into an HTTP header value.
///
/// Returns an error response if conversion fails.
#[allow(clippy::needless_pass_by_value)] // settlement is consumed by serialization
fn settlement_to_header(settlement: proto::SettleResponse) -> Result<HeaderValue, PaygateError> {
    let json =
        serde_json::to_vec(&settlement).map_err(|err| PaygateError::Settlement(err.to_string()))?;
    let payment_header = Base64Bytes::encode(json);
    HeaderValue::from_bytes(payment_header.as_ref())
        .map_err(|err| PaygateError::Settlement(err.to_string()))
}
