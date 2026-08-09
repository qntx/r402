//! Client-side x402 payment handling for reqwest.
//!
//! [`X402Client`] is a thin HTTP adapter over [`r402_core::PaymentClient`]:
//! scheme registration, policies, selection, and lifecycle hooks live in core.
//! This module maps signed payloads to `Payment-Signature` and dispatches
//! `on_payment_response` after paid retries (including one corrective recovery).

use http::{Extensions, HeaderMap, StatusCode};
use r402_core::ClientHooks;
use r402_core::client::{CreatedPayment, PaymentClient, PaymentResponseContext};
use r402_core::error::ClientError;
use r402_core::scheme::{FirstMatch, PaymentPolicy, PaymentSelector, SchemeClient};
use r402_core::wire;
use r402_core::wire::Base64Bytes;
use reqwest::{Request, Response};
use reqwest_middleware as rqm;
#[cfg(feature = "telemetry")]
use tracing::{debug, info, instrument, trace};

/// HTTP middleware over a core [`PaymentClient`].
///
/// Automatically handles `402 Payment Required` by signing a payment and
/// retrying once (or twice if `on_payment_response` signals recovery).
#[allow(
    missing_debug_implementations,
    reason = "PaymentClient contains dyn trait objects"
)]
pub struct X402Client<TSelector = FirstMatch> {
    inner: PaymentClient<TSelector>,
}

impl X402Client<FirstMatch> {
    /// Creates a new client with [`FirstMatch`] selection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for X402Client<FirstMatch> {
    fn default() -> Self {
        Self {
            inner: PaymentClient::new(),
        }
    }
}

impl<TSelector> X402Client<TSelector> {
    /// Builds from an existing core payment client.
    #[must_use]
    pub const fn from_payment_client(inner: PaymentClient<TSelector>) -> Self {
        Self { inner }
    }

    /// Returns a shared reference to the core payment client.
    #[must_use]
    pub const fn payment_client(&self) -> &PaymentClient<TSelector> {
        &self.inner
    }

    /// Registers a scheme client.
    #[must_use]
    pub fn register<S>(mut self, scheme: S) -> Self
    where
        S: SchemeClient + 'static,
    {
        self.inner = self.inner.register(scheme);
        self
    }

    /// Sets a custom payment selector.
    #[must_use]
    pub fn with_selector<P: PaymentSelector>(self, selector: P) -> X402Client<P> {
        X402Client {
            inner: self.inner.with_selector(selector),
        }
    }

    /// Adds a payment policy.
    #[must_use]
    pub fn with_policy<P: PaymentPolicy + 'static>(mut self, policy: P) -> Self {
        self.inner = self.inner.with_policy(policy);
        self
    }

    /// Adds a client lifecycle hook (including `on_payment_response`).
    #[must_use]
    pub fn with_hook(mut self, hook: impl ClientHooks + 'static) -> Self {
        self.inner = self.inner.with_hook(hook);
        self
    }
}

impl<TSelector> X402Client<TSelector>
where
    TSelector: PaymentSelector,
{
    /// Creates `Payment-Signature` headers from a 402 response.
    ///
    /// # Errors
    ///
    /// Parse, selection, signing, or before-hook abort.
    #[cfg_attr(
        feature = "telemetry",
        instrument(name = "x402.reqwest.make_payment_headers", skip_all, err)
    )]
    pub async fn make_payment_headers(&self, res: Response) -> Result<HeaderMap, ClientError> {
        let payment_required = parse_payment_required(res)
            .await
            .ok_or_else(|| ClientError::Parse("Invalid 402 response".to_owned()))?;
        let created = self.inner.create_payment(&payment_required).await?;
        Ok(payment_signature_headers(&created))
    }

    /// Creates a payment for an already-parsed challenge.
    ///
    /// # Errors
    ///
    /// Selection, signing, or before-hook abort.
    pub async fn create_payment(
        &self,
        payment_required: &wire::PaymentRequired,
    ) -> Result<CreatedPayment, ClientError> {
        self.inner.create_payment(payment_required).await
    }
}

/// Builds the `Payment-Signature` header map from a created payment.
///
/// # Panics
///
/// Panics if the signed payload is not a valid HTTP header value. Signed
/// base64 payloads from scheme clients are always ASCII.
#[must_use]
pub fn payment_signature_headers(created: &CreatedPayment) -> HeaderMap {
    let mut headers = HeaderMap::new();
    #[allow(
        clippy::expect_used,
        reason = "base64-encoded payload is always valid ASCII header"
    )]
    headers.insert(
        "Payment-Signature",
        created
            .signed_payload
            .parse()
            .expect("signed payload is valid header value"),
    );
    headers
}

/// Runs the next middleware or HTTP client with optional telemetry instrumentation.
#[cfg_attr(
    feature = "telemetry",
    instrument(name = "x402.reqwest.next", skip_all)
)]
async fn run_next(
    next: rqm::Next<'_>,
    req: Request,
    extensions: &mut Extensions,
) -> rqm::Result<Response> {
    next.run(req, extensions).await
}

#[async_trait::async_trait]
impl<TSelector> rqm::Middleware for X402Client<TSelector>
where
    TSelector: PaymentSelector + Send + Sync + 'static,
{
    /// Handles a request, automatically handling 402 responses.
    ///
    /// When a 402 is received:
    /// 1. Extract requirements, create signed payment (hooks)
    /// 2. Retry with `Payment-Signature`
    /// 3. Dispatch `on_payment_response` on settle / corrective 402
    /// 4. If recovered, rebuild payment and retry once more
    #[cfg_attr(
        feature = "telemetry",
        instrument(name = "x402.reqwest.handle", skip_all, err)
    )]
    async fn handle(
        &self,
        req: Request,
        extensions: &mut Extensions,
        next: rqm::Next<'_>,
    ) -> rqm::Result<Response> {
        let retry_template = req.try_clone();
        let res = run_next(next.clone(), req, extensions).await?;

        if res.status() != StatusCode::PAYMENT_REQUIRED {
            #[cfg(feature = "telemetry")]
            trace!(status = ?res.status(), "No payment required, returning response");
            return Ok(res);
        }

        #[cfg(feature = "telemetry")]
        info!(url = ?res.url(), "Received 402 Payment Required, processing payment");

        let Some(template) = retry_template else {
            #[cfg(feature = "telemetry")]
            tracing::warn!("Cannot auto-retry 402: request body not cloneable, returning raw 402");
            return Ok(res);
        };

        let payment_required = parse_payment_required(res).await.ok_or_else(|| {
            rqm::Error::Middleware(ClientError::Parse("Invalid 402 response".into()).into())
        })?;

        let created = self
            .inner
            .create_payment(&payment_required)
            .await
            .map_err(|e| rqm::Error::Middleware(e.into()))?;

        let paid = clone_with_payment(&template, &created)
            .map_err(|e| rqm::Error::Middleware(e.into()))?;

        #[cfg(feature = "telemetry")]
        trace!(url = ?paid.url(), "Retrying request with payment headers");

        let response = run_next(next.clone(), paid, extensions).await?;

        let Some(recovered) = self
            .dispatch_and_maybe_recover(&payment_required, &created, &response)
            .await
            .map_err(|e| rqm::Error::Middleware(e.into()))?
        else {
            return Ok(response);
        };

        let second = clone_with_payment(&template, &recovered)
            .map_err(|e| rqm::Error::Middleware(e.into()))?;
        let second_response = run_next(next, second, extensions).await?;
        // Fire hooks on the second response without further recovery.
        let ctx = build_response_context(&payment_required, &recovered, &second_response);
        if ctx.settle_response.is_some() || ctx.corrective_payment_required.is_some() {
            let _ = self.inner.handle_payment_response(&ctx).await;
        }
        Ok(second_response)
    }
}

impl<TSelector: PaymentSelector> X402Client<TSelector> {
    /// Returns `Some(new_created)` when hooks signal recovery and a fresh
    /// payment can be built from the corrective challenge (or original).
    async fn dispatch_and_maybe_recover(
        &self,
        original: &wire::PaymentRequired,
        created: &CreatedPayment,
        response: &Response,
    ) -> Result<Option<CreatedPayment>, ClientError> {
        let ctx = build_response_context(original, created, response);
        if ctx.settle_response.is_none() && ctx.corrective_payment_required.is_none() {
            return Ok(None);
        }

        let result = self.inner.handle_payment_response(&ctx).await;
        if !result.recovered {
            return Ok(None);
        }

        let challenge = ctx.corrective_payment_required.as_ref().unwrap_or(original);
        let fresh = self.inner.create_payment(challenge).await?;
        Ok(Some(fresh))
    }
}

fn build_response_context(
    original: &wire::PaymentRequired,
    created: &CreatedPayment,
    response: &Response,
) -> PaymentResponseContext {
    let settle_response = response
        .headers()
        .get("Payment-Response")
        .and_then(|h| Base64Bytes::from(h.as_bytes()).decode().ok())
        .and_then(|b| serde_json::from_slice(&b).ok());

    let corrective_payment_required =
        if settle_response.is_none() && response.status() == StatusCode::PAYMENT_REQUIRED {
            response
                .headers()
                .get("Payment-Required")
                .and_then(|h| Base64Bytes::from(h.as_bytes()).decode().ok())
                .and_then(|b| serde_json::from_slice(&b).ok())
        } else {
            None
        };

    let mut ctx = PaymentResponseContext::new(original.clone(), created.signed_payload.clone());
    if let Some(settle) = settle_response {
        ctx = ctx.with_settle_response(settle);
    }
    if let Some(required) = corrective_payment_required {
        ctx = ctx.with_corrective_payment_required(required);
    }
    ctx
}

fn clone_with_payment(
    template: &Request,
    created: &CreatedPayment,
) -> Result<Request, ClientError> {
    let mut req = template
        .try_clone()
        .ok_or(ClientError::RequestNotCloneable)?;
    req.headers_mut().extend(payment_signature_headers(created));
    Ok(req)
}

/// Parses a 402 Payment Required response into a [`wire::PaymentRequired`].
///
/// Tries `Payment-Required` header (base64 JSON) first, then response body.
#[cfg_attr(
    feature = "telemetry",
    instrument(name = "x402.reqwest.parse_payment_required", skip(response))
)]
pub async fn parse_payment_required(response: Response) -> Option<wire::PaymentRequired> {
    let v2_from_header = response
        .headers()
        .get("Payment-Required")
        .and_then(|h| Base64Bytes::from(h.as_bytes()).decode().ok())
        .and_then(|b| serde_json::from_slice::<wire::PaymentRequired>(&b).ok());

    if let Some(v2_payment_required) = v2_from_header {
        #[cfg(feature = "telemetry")]
        debug!("Parsed V2 payment required from header");
        return Some(v2_payment_required);
    }

    if let Ok(body_bytes) = response.bytes().await
        && let Ok(v2_from_body) = serde_json::from_slice::<wire::PaymentRequired>(&body_bytes)
    {
        #[cfg(feature = "telemetry")]
        debug!("Parsed V2 payment required from response body");
        return Some(v2_from_body);
    }

    #[cfg(feature = "telemetry")]
    debug!("Could not parse payment required from response");

    None
}
