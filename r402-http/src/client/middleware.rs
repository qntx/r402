//! Client-side x402 payment handling for reqwest.
//!
//! This module provides the [`X402Client`] which orchestrates scheme clients
//! and payment selection for automatic payment handling.

use http::{Extensions, HeaderMap, StatusCode};
use r402::encoding::Base64Bytes;
use r402::proto;
use r402::proto::{v1, v2};
use r402::scheme::{
    FirstMatch, PaymentCandidate, PaymentPolicy, PaymentSelector, X402Error, X402SchemeClient,
};
use reqwest::{Request, Response};
use reqwest_middleware as rqm;
use std::sync::Arc;

use super::hooks::{
    ClientHooks, PaymentCreatedContext, PaymentCreationContext, PaymentCreationFailureContext,
};

#[cfg(feature = "telemetry")]
use tracing::{debug, info, instrument, trace};

/// The main x402 client that orchestrates scheme clients and selection.
///
/// The [`X402Client`] acts as middleware for reqwest, automatically handling
/// 402 Payment Required responses by extracting payment requirements, signing
/// payments, and retrying requests.
#[allow(missing_debug_implementations)] // ClientSchemes contains dyn trait objects
pub struct X402Client<TSelector> {
    schemes: ClientSchemes,
    selector: TSelector,
    policies: Vec<Arc<dyn PaymentPolicy>>,
    hooks: Arc<ClientHooks>,
}

impl X402Client<FirstMatch> {
    /// Creates a new [`X402Client`] with default settings.
    ///
    /// The default client uses [`FirstMatch`] payment selection, which selects
    /// the first matching payment scheme.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for X402Client<FirstMatch> {
    fn default() -> Self {
        Self {
            schemes: ClientSchemes::default(),
            selector: FirstMatch,
            policies: Vec::new(),
            hooks: Arc::new(ClientHooks::default()),
        }
    }
}

impl<TSelector> X402Client<TSelector> {
    /// Registers a scheme client for specific chains or networks.
    ///
    /// Scheme clients handle the actual payment signing for specific protocols.
    /// You can register multiple clients for different chains or schemes.
    ///
    /// # Arguments
    ///
    /// * `scheme` - The scheme client implementation to register
    ///
    /// # Returns
    ///
    /// A new [`X402Client`] with the additional scheme registered.
    #[must_use]
    pub fn register<S>(mut self, scheme: S) -> Self
    where
        S: X402SchemeClient + 'static,
    {
        self.schemes.push(scheme);
        self
    }

    /// Sets a custom payment selector.
    ///
    /// By default, [`FirstMatch`] is used which selects the first matching scheme.
    /// You can implement custom selection logic by providing your own [`PaymentSelector`].
    pub fn with_selector<P: PaymentSelector + 'static>(self, selector: P) -> X402Client<P> {
        X402Client {
            selector,
            schemes: self.schemes,
            policies: self.policies,
            hooks: self.hooks,
        }
    }

    /// Adds a payment policy to the filtering pipeline.
    ///
    /// Policies are applied in registration order before the selector picks
    /// the final candidate. Use policies to restrict which networks, schemes,
    /// or amounts are acceptable.
    #[must_use]
    pub fn with_policy<P: PaymentPolicy + 'static>(mut self, policy: P) -> Self {
        self.policies.push(Arc::new(policy));
        self
    }

    /// Sets the lifecycle hooks for payment creation.
    ///
    /// Hooks allow intercepting the payment creation pipeline for logging,
    /// custom validation, or error recovery.
    #[must_use]
    pub fn with_hooks(mut self, hooks: ClientHooks) -> Self {
        self.hooks = Arc::new(hooks);
        self
    }
}

impl<TSelector> X402Client<TSelector>
where
    TSelector: PaymentSelector,
{
    /// Creates payment headers from a 402 response.
    ///
    /// This method extracts the payment requirements from the response,
    /// selects the best payment option, signs the payment, and returns
    /// the appropriate headers to include in the retry request.
    ///
    /// # Arguments
    ///
    /// * `res` - The 402 Payment Required response
    ///
    /// # Returns
    ///
    /// A [`HeaderMap`] containing the payment signature header, or an error.
    ///
    /// # Errors
    ///
    /// Returns [`X402Error::ParseError`] if the response cannot be parsed.
    /// Returns [`X402Error::NoMatchingPaymentOption`] if no registered scheme
    /// can handle the payment requirements.
    ///
    /// # Panics
    ///
    /// Panics if the signed payload is not a valid HTTP header value.
    #[cfg_attr(
        feature = "telemetry",
        instrument(name = "x402.reqwest.make_payment_headers", skip_all, err)
    )]
    pub async fn make_payment_headers(&self, res: Response) -> Result<HeaderMap, X402Error> {
        let payment_required = parse_payment_required(res)
            .await
            .ok_or_else(|| X402Error::ParseError("Invalid 402 response".to_string()))?;

        let hook_ctx = PaymentCreationContext {
            payment_required: payment_required.clone(),
        };

        // Execute before-payment-creation hooks
        for hook in &self.hooks.before_payment_creation {
            if let Ok(Some(result)) = hook(hook_ctx.clone()).await
                && result.abort
            {
                return Err(X402Error::ParseError(result.reason));
            }
        }

        let creation_result = self.create_payment_headers_inner(&payment_required).await;

        match creation_result {
            Ok(headers) => {
                // Execute after-payment-creation hooks (errors logged, not propagated)
                let result_ctx = PaymentCreatedContext {
                    ctx: hook_ctx,
                    headers: headers.clone(),
                };
                for hook in &self.hooks.after_payment_creation {
                    let _ = hook(result_ctx.clone()).await;
                }
                Ok(headers)
            }
            Err(err) => {
                // Execute on-payment-creation-failure hooks (may recover)
                let failure_ctx = PaymentCreationFailureContext {
                    ctx: hook_ctx,
                    error: err.to_string(),
                };
                for hook in &self.hooks.on_payment_creation_failure {
                    if let Ok(Some(result)) = hook(failure_ctx.clone()).await
                        && result.recovered
                    {
                        return Ok(result.headers);
                    }
                }
                Err(err)
            }
        }
    }

    /// Internal helper that performs the actual payment header creation.
    async fn create_payment_headers_inner(
        &self,
        payment_required: &proto::PaymentRequired,
    ) -> Result<HeaderMap, X402Error> {
        let candidates = self.schemes.candidates(payment_required);

        // Apply policies to filter candidates
        let mut filtered: Vec<&PaymentCandidate> = candidates.iter().collect();
        for policy in &self.policies {
            filtered = policy.apply(filtered);
            if filtered.is_empty() {
                return Err(X402Error::NoMatchingPaymentOption);
            }
        }

        // Select the best candidate from filtered list
        let selected = self
            .selector
            .select(&filtered)
            .ok_or(X402Error::NoMatchingPaymentOption)?;

        #[cfg(feature = "telemetry")]
        debug!(
            scheme = %selected.scheme,
            chain_id = %selected.chain_id,
            "Selected payment scheme"
        );

        let signed_payload = selected.sign().await?;
        let header_name = match payment_required {
            proto::PaymentRequired::V1(_) => "X-Payment",
            _ => "Payment-Signature",
        };
        let headers = {
            let mut headers = HeaderMap::new();
            headers.insert(
                header_name,
                signed_payload
                    .parse()
                    .expect("signed payload is valid header value"),
            );
            headers
        };

        Ok(headers)
    }
}

/// Internal collection of registered scheme clients.
#[derive(Default)]
#[allow(missing_debug_implementations)] // dyn trait objects do not implement Debug
pub struct ClientSchemes(Vec<Arc<dyn X402SchemeClient>>);

impl ClientSchemes {
    /// Adds a scheme client to the collection.
    pub fn push<T: X402SchemeClient + 'static>(&mut self, client: T) {
        self.0.push(Arc::new(client));
    }

    /// Finds all payment candidates that can handle the given payment requirements.
    #[must_use]
    pub fn candidates(&self, payment_required: &proto::PaymentRequired) -> Vec<PaymentCandidate> {
        let mut candidates = vec![];
        for client in &self.0 {
            let accepted = client.accept(payment_required);
            candidates.extend(accepted);
        }
        candidates
    }
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
    /// When a 402 response is received, this middleware:
    /// 1. Extracts payment requirements from the response
    /// 2. Signs a payment using registered scheme clients
    /// 3. Retries the request with the payment header
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
        let retry_req = req.try_clone();
        let res = run_next(next.clone(), req, extensions).await?;

        if res.status() != StatusCode::PAYMENT_REQUIRED {
            #[cfg(feature = "telemetry")]
            trace!(status = ?res.status(), "No payment required, returning response");
            return Ok(res);
        }

        #[cfg(feature = "telemetry")]
        info!(url = ?res.url(), "Received 402 Payment Required, processing payment");

        let headers = self
            .make_payment_headers(res)
            .await
            .map_err(|e| rqm::Error::Middleware(e.into()))?;

        // Retry with payment
        let mut retry = retry_req.ok_or(rqm::Error::Middleware(
            X402Error::RequestNotCloneable.into(),
        ))?;
        retry.headers_mut().extend(headers);

        #[cfg(feature = "telemetry")]
        trace!(url = ?retry.url(), "Retrying request with payment headers");

        run_next(next, retry, extensions).await
    }
}

/// Parses a 402 Payment Required response into a [`proto::PaymentRequired`].
///
/// Supports both V1 (JSON body) and V2 (base64-encoded header) formats.
#[cfg_attr(
    feature = "telemetry",
    instrument(name = "x402.reqwest.parse_payment_required", skip(response))
)]
pub async fn parse_payment_required(response: Response) -> Option<proto::PaymentRequired> {
    // Try V2 format first (header-based)
    let headers = response.headers();
    let v2_payment_required = headers
        .get("Payment-Required")
        .and_then(|h| Base64Bytes::from(h.as_bytes()).decode().ok())
        .and_then(|b| serde_json::from_slice::<v2::PaymentRequired>(&b).ok());
    if let Some(v2_payment_required) = v2_payment_required {
        #[cfg(feature = "telemetry")]
        debug!("Parsed V2 payment required from header");
        return Some(proto::PaymentRequired::V2(v2_payment_required));
    }

    // Fall back to V1 format (body-based)
    let v1_payment_required = response
        .bytes()
        .await
        .ok()
        .and_then(|b| serde_json::from_slice::<v1::PaymentRequired>(&b).ok());
    if let Some(v1_payment_required) = v1_payment_required {
        #[cfg(feature = "telemetry")]
        debug!("Parsed V1 payment required from body");
        return Some(proto::PaymentRequired::V1(v1_payment_required));
    }

    #[cfg(feature = "telemetry")]
    debug!("Could not parse payment required from response");

    None
}
