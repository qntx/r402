//! HTTP client middleware for automatic x402 payment handling.
//!
//! Provides [`X402HttpClient`] which implements [`reqwest_middleware::Middleware`]
//! to automatically intercept 402 responses, create payment payloads via an
//! [`r402::client::X402Client`], and retry with the `PAYMENT-SIGNATURE` header.
//!
//! Corresponds to Python SDK's `http/x402_http_client.py` +
//! `http/x402_http_client_base.py`.

use std::future::Future;
use std::sync::Arc;

use r402::client::X402Client;
use r402::proto::{PaymentPayload, PaymentPayloadV1, PaymentRequired, PaymentRequiredV1};
use reqwest::{Request, Response};
use reqwest_middleware::{Middleware, Next};

use crate::constants::{PAYMENT_REQUIRED_HEADER, PAYMENT_SIGNATURE_HEADER, X_PAYMENT_HEADER};
use crate::error::HttpError;
use crate::headers::{decode_payment_required, encode_payment_signature, encode_x_payment};

/// reqwest-middleware that automatically handles HTTP 402 responses.
///
/// When a response with status 402 is received, the middleware:
/// 1. Decodes the `PAYMENT-REQUIRED` header (or V1 body)
/// 2. Delegates to [`X402Client::create_payment_payload`] to build a signed payload
/// 3. Retries the request with the `PAYMENT-SIGNATURE` header attached
///
/// # Example
///
/// ```no_run
/// use std::sync::Arc;
/// use r402::client::X402Client;
/// use r402_http::client::X402HttpClient;
/// use reqwest_middleware::ClientBuilder;
///
/// let x402_client = Arc::new(X402Client::new());
/// // Register scheme clients on x402_client...
///
/// let http_client = ClientBuilder::new(reqwest::Client::new())
///     .with(X402HttpClient::new(x402_client))
///     .build();
/// ```
///
/// Corresponds to Python SDK's `x402HTTPClient` + `PaymentRoundTripper`.
#[derive(Debug, Clone)]
pub struct X402HttpClient {
    client: Arc<X402Client>,
}

impl X402HttpClient {
    /// Creates a new middleware wrapping the given x402 client.
    #[must_use]
    pub fn new(client: Arc<X402Client>) -> Self {
        Self { client }
    }

    /// Creates a new middleware from an owned [`X402Client`].
    ///
    /// Convenience wrapper that wraps the client in an [`Arc`] internally.
    #[must_use]
    pub fn from_client(client: X402Client) -> Self {
        Self {
            client: Arc::new(client),
        }
    }

    /// Builds a [`reqwest_middleware::ClientWithMiddleware`] with x402 payment
    /// handling from an owned [`X402Client`].
    ///
    /// This is the simplest way to get a payment-capable HTTP client:
    ///
    /// ```ignore
    /// use r402::client::X402Client;
    /// use r402_http::client::X402HttpClient;
    ///
    /// let http_client = X402HttpClient::build_reqwest(
    ///     X402Client::builder()
    ///         .register("eip155:*".into(), Box::new(evm_scheme))
    ///         .build()
    /// );
    /// ```
    #[must_use]
    pub fn build_reqwest(client: X402Client) -> reqwest_middleware::ClientWithMiddleware {
        reqwest_middleware::ClientBuilder::new(reqwest::Client::new())
            .with(Self::from_client(client))
            .build()
    }

    /// Extracts payment-required info from a 402 response.
    ///
    /// Checks V2 header first, then falls back to V1 body.
    async fn extract_payment_required(response: &Response) -> Option<PaymentRequiredVersion> {
        // V2: PAYMENT-REQUIRED header
        if let Some(header_value) = response.headers().get(PAYMENT_REQUIRED_HEADER) {
            if let Ok(s) = header_value.to_str() {
                if let Ok(parsed) = decode_payment_required(s) {
                    return match parsed {
                        r402::proto::helpers::PaymentRequiredEnum::V2(pr) => {
                            Some(PaymentRequiredVersion::V2(*pr))
                        }
                        r402::proto::helpers::PaymentRequiredEnum::V1(pr) => {
                            Some(PaymentRequiredVersion::V1(*pr))
                        }
                    };
                }
            }
        }

        None
    }

    /// Encodes a payment payload into the appropriate HTTP header.
    fn encode_payment_header(
        payload: &PaymentPayloadVersion,
    ) -> Result<(String, String), HttpError> {
        match payload {
            PaymentPayloadVersion::V2(p) => {
                let encoded = encode_payment_signature(p)?;
                Ok((PAYMENT_SIGNATURE_HEADER.to_owned(), encoded))
            }
            PaymentPayloadVersion::V1(p) => {
                let encoded = encode_x_payment(p)?;
                Ok((X_PAYMENT_HEADER.to_owned(), encoded))
            }
        }
    }
}

/// Internal version-tagged payment required.
enum PaymentRequiredVersion {
    V2(PaymentRequired),
    V1(PaymentRequiredV1),
}

/// Internal version-tagged payment payload.
enum PaymentPayloadVersion {
    V2(PaymentPayload),
    V1(PaymentPayloadV1),
}

impl Middleware for X402HttpClient {
    fn handle<'life0, 'life1, 'life2, 'async_trait>(
        &'life0 self,
        req: Request,
        extensions: &'life1 mut http::Extensions,
        next: Next<'life2>,
    ) -> core::pin::Pin<
        Box<dyn Future<Output = Result<Response, reqwest_middleware::Error>> + Send + 'async_trait>,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            // Clone request info for potential retry
            let method = req.method().clone();
            let url = req.url().clone();
            let original_headers = req.headers().clone();

            // Send original request
            let response = next.clone().run(req, extensions).await?;

            // Not a 402 — pass through
            if response.status().as_u16() != 402 {
                return Ok(response);
            }

            // Extract payment requirements from the 402 response
            let payment_required = match Self::extract_payment_required(&response).await {
                Some(pr) => pr,
                None => return Ok(response),
            };

            // Create payment payload via x402 client
            let payment_payload = match &payment_required {
                PaymentRequiredVersion::V2(pr) => {
                    match self.client.create_payment_payload(pr).await {
                        Ok(p) => PaymentPayloadVersion::V2(p),
                        Err(_) => return Ok(response),
                    }
                }
                PaymentRequiredVersion::V1(pr) => {
                    match self.client.create_payment_payload_v1(pr).await {
                        Ok(p) => PaymentPayloadVersion::V1(p),
                        Err(_) => return Ok(response),
                    }
                }
            };

            // Encode payment into header
            let (header_name, header_value) = match Self::encode_payment_header(&payment_payload) {
                Ok(h) => h,
                Err(_) => return Ok(response),
            };

            // Build retry request with payment header
            let mut retry_req = Request::new(method, url);
            *retry_req.headers_mut() = original_headers;
            retry_req.headers_mut().insert(
                reqwest::header::HeaderName::from_bytes(header_name.as_bytes())
                    .expect("valid header name"),
                reqwest::header::HeaderValue::from_str(&header_value).expect("valid header value"),
            );

            // Send retry
            next.run(retry_req, extensions).await
        })
    }
}
