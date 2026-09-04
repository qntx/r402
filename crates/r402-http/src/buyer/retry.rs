//! Paid retry is HTTP 402 only. 412 and 502 are returned as-is.

use std::fmt::{self, Debug, Formatter};

use http::{Extensions, HeaderMap, StatusCode};
use r402_client::{
    ClientExtension, ClientHooks, CreatedPayment, FirstMatch, PaymentClient, PaymentPolicy,
    PaymentResponseContext, PaymentSelector, SchemeClient, SpendControls,
};
use r402_protocol::ClientError;
use r402_protocol::payment::PaymentRequired;
use reqwest::{Client, Request, Response};
use reqwest_middleware as rqm;

use super::signature::{
    clone_with_payment, parse_payment_required, payment_required_from_headers,
    payment_signature_headers, settle_response_from_headers,
};

/// HTTP middleware over a core [`PaymentClient`].
///
/// On `402 Payment Required`, signs a payment and retries once (or twice if
/// `on_payment_response` signals recovery). Other statuses, including 412 and
/// 502, are never retried.
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

impl<TSelector> Debug for X402Client<TSelector> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("X402Client")
            .field("inner", &self.inner)
            .finish()
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
    pub fn register(mut self, scheme: impl SchemeClient + 'static) -> Self {
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
    pub fn with_policy(mut self, policy: impl PaymentPolicy + 'static) -> Self {
        self.inner = self.inner.with_policy(policy);
        self
    }

    /// Adds a client lifecycle hook (including `on_payment_response`).
    #[must_use]
    pub fn with_hook(mut self, hook: impl ClientHooks + 'static) -> Self {
        self.inner = self.inner.with_hook(hook);
        self
    }

    /// Adds a client extension (HTTP 402 header hook and payload enrich).
    #[must_use]
    pub fn with_extension(mut self, extension: impl ClientExtension + 'static) -> Self {
        self.inner = self.inner.with_extension(extension);
        self
    }

    /// Enables spend controls with the given configuration.
    #[must_use]
    pub fn with_spend_controls(mut self, controls: SpendControls) -> Self {
        self.inner = self.inner.with_spend_controls(controls);
        self
    }

    /// Disables all spend controls (any asset, no caps).
    #[must_use]
    pub fn disable_spend_controls(mut self) -> Self {
        self.inner = self.inner.disable_spend_controls();
        self
    }
}

impl<TSelector> X402Client<TSelector>
where
    TSelector: PaymentSelector,
{
    /// Creates `Payment-Signature` plus advertised extension headers from a 402.
    ///
    /// `SIGN-IN-WITH-X` is included when a registered extension's `key` is in
    /// `PaymentRequired.extensions`.
    ///
    /// # Errors
    ///
    /// Parse, selection, signing, or before-hook abort.
    pub async fn make_payment_headers(&self, res: Response) -> Result<HeaderMap, ClientError> {
        let request_url = res.url().clone();
        let payment_required = parse_payment_required(res)
            .await
            .ok_or_else(|| ClientError::Parse("Invalid 402 response".to_owned()))?;
        let created = self.inner.create_payment(&payment_required).await?;
        let mut headers = payment_signature_headers(&created)?;
        headers.extend(
            self.inner
                .extension_headers(&payment_required, request_url.as_str())
                .await,
        );
        Ok(headers)
    }

    /// Creates a payment for an already-parsed challenge.
    ///
    /// # Errors
    ///
    /// Selection, signing, or before-hook abort.
    pub async fn create_payment(
        &self,
        payment_required: &PaymentRequired,
    ) -> Result<CreatedPayment, ClientError> {
        self.inner.create_payment(payment_required).await
    }
}

impl<S> X402Client<S>
where
    Self: rqm::Middleware,
{
    /// Wraps a reqwest [`Client`] with x402 payment middleware.
    #[must_use]
    pub fn wrap(self, client: Client) -> rqm::ClientWithMiddleware {
        rqm::ClientBuilder::new(client).with(self).build()
    }

    /// Wraps a reqwest [`Client`] and returns the middleware builder.
    #[must_use]
    pub fn wrap_builder(self, client: Client) -> rqm::ClientBuilder {
        rqm::ClientBuilder::new(client).with(self)
    }
}

/// Extension trait for adding x402 payment handling to a reqwest [`Client`].
pub trait WithPayments {
    /// Adds x402 payment middleware, returning a ready-to-use client.
    #[must_use]
    fn with_payments<S>(self, x402: X402Client<S>) -> rqm::ClientWithMiddleware
    where
        X402Client<S>: rqm::Middleware;
}

impl WithPayments for Client {
    fn with_payments<S>(self, x402: X402Client<S>) -> rqm::ClientWithMiddleware
    where
        X402Client<S>: rqm::Middleware,
    {
        x402.wrap(self)
    }
}

/// Paid auto-retry is 402 only. Permit2 412 and facilitator transport 502
/// must not be retried.
fn should_retry(status: StatusCode) -> bool {
    status == StatusCode::PAYMENT_REQUIRED
}

#[async_trait::async_trait]
impl<TSelector> rqm::Middleware for X402Client<TSelector>
where
    TSelector: PaymentSelector + Send + Sync + 'static,
{
    async fn handle(
        &self,
        req: Request,
        extensions: &mut Extensions,
        next: rqm::Next<'_>,
    ) -> rqm::Result<Response> {
        let retry_template = req.try_clone();
        let res = next.clone().run(req, extensions).await?;
        if !should_retry(res.status()) {
            return Ok(res);
        }

        let Some(template) = retry_template else {
            return Ok(res);
        };

        let request_url = res.url().clone();
        let payment_required = parse_payment_required(res)
            .await
            .ok_or_else(|| middleware_err(ClientError::Parse("Invalid 402 response".into())))?;
        let created = self
            .inner
            .create_payment(&payment_required)
            .await
            .map_err(middleware_err)?;
        let paid = clone_with_payment_and_extensions(
            &template,
            &created,
            &payment_required,
            request_url.as_str(),
            &self.inner,
        )
        .await
        .map_err(middleware_err)?;
        let response = next.clone().run(paid, extensions).await?;

        let Some(recovered) = self
            .dispatch_and_maybe_recover(&payment_required, &created, &response)
            .await
            .map_err(middleware_err)?
        else {
            return Ok(response);
        };

        let second = clone_with_payment_and_extensions(
            &template,
            &recovered,
            &recovered.payment_required,
            response.url().as_str(),
            &self.inner,
        )
        .await
        .map_err(middleware_err)?;
        let second_response = next.run(second, extensions).await?;
        let ctx = build_response_context(&payment_required, &recovered, &second_response);
        if ctx.settle_response.is_some() || ctx.corrective_payment_required.is_some() {
            self.inner.handle_payment_response(&ctx).await;
        }
        Ok(second_response)
    }
}

impl<TSelector: PaymentSelector> X402Client<TSelector> {
    /// Returns `Some(new_created)` when hooks signal recovery and a fresh
    /// payment can be built from the corrective challenge (or original).
    async fn dispatch_and_maybe_recover(
        &self,
        original: &PaymentRequired,
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

async fn clone_with_payment_and_extensions<TSelector: PaymentSelector>(
    template: &Request,
    created: &CreatedPayment,
    payment_required: &PaymentRequired,
    request_url: &str,
    client: &PaymentClient<TSelector>,
) -> Result<Request, ClientError> {
    let mut paid = clone_with_payment(template, created)?;
    paid.headers_mut().extend(
        client
            .extension_headers(payment_required, request_url)
            .await,
    );
    Ok(paid)
}

fn build_response_context(
    original: &PaymentRequired,
    created: &CreatedPayment,
    response: &Response,
) -> PaymentResponseContext {
    let settle_response = settle_response_from_headers(response.headers());
    let corrective_payment_required =
        if settle_response.is_none() && should_retry(response.status()) {
            payment_required_from_headers(response.headers())
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

fn middleware_err(err: ClientError) -> rqm::Error {
    rqm::Error::middleware(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_only_status_402() {
        assert!(
            should_retry(StatusCode::PAYMENT_REQUIRED),
            "402 is the only auto-retry status"
        );
        assert!(
            !should_retry(StatusCode::PRECONDITION_FAILED),
            "412 Permit2 must not retry"
        );
        assert!(
            !should_retry(StatusCode::BAD_GATEWAY),
            "502 transport must not retry"
        );
        assert!(
            !should_retry(StatusCode::TOO_MANY_REQUESTS),
            "429 is facilitator /supported, not buyer retry"
        );
        assert!(!should_retry(StatusCode::OK), "200 must not retry");
    }
}
