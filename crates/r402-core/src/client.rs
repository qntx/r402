//! Client-side payment orchestration (official `x402Client`, V2-only).
//!
//! Registers scheme clients, applies policies, selects a candidate, signs,
//! and runs lifecycle hooks including **`on_payment_response`**.
//!
//! Transports (HTTP / MCP) wrap this type; they do not re-implement selection.

use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::error::ClientError;
use crate::facilitator::{BoxFuture, FailureRecovery, HookDecision};
use crate::scheme::{FirstMatch, PaymentCandidate, PaymentPolicy, PaymentSelector, SchemeClient};
use crate::wire::{PaymentRequired, SettleResponse};

/// Context for payment-creation hooks.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PaymentCreationContext {
    /// Parsed payment requirements from the 402 challenge.
    pub payment_required: PaymentRequired,
}

impl PaymentCreationContext {
    /// Constructs a creation context from a 402 challenge.
    #[must_use]
    pub const fn new(payment_required: PaymentRequired) -> Self {
        Self { payment_required }
    }
}

/// Result of successful payment creation (transport-agnostic).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CreatedPayment {
    /// Base64-encoded payment payload for `Payment-Signature` / MCP meta.
    pub signed_payload: String,
    /// Challenge that was paid.
    pub payment_required: PaymentRequired,
}

impl CreatedPayment {
    /// Constructs a created-payment record.
    #[must_use]
    pub fn new(signed_payload: impl Into<String>, payment_required: PaymentRequired) -> Self {
        Self {
            signed_payload: signed_payload.into(),
            payment_required,
        }
    }
}

/// Context delivered after a paid request completes.
///
/// Official semantics: exactly one of `settle_response` or
/// `corrective_payment_required` is typically set —
/// - settle: paid request succeeded with `Payment-Response`
/// - corrective 402: server rejected with a new `Payment-Required`
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PaymentResponseContext {
    /// Original 402 challenge used to build the payment.
    pub payment_required: PaymentRequired,
    /// Signed payload that was submitted.
    pub signed_payload: String,
    /// Parsed settle outcome when present.
    pub settle_response: Option<SettleResponse>,
    /// Corrective `Payment-Required` when the paid retry returned 402.
    pub corrective_payment_required: Option<PaymentRequired>,
}

impl PaymentResponseContext {
    /// Constructs a payment-response context.
    #[must_use]
    pub fn new(payment_required: PaymentRequired, signed_payload: impl Into<String>) -> Self {
        Self {
            payment_required,
            signed_payload: signed_payload.into(),
            settle_response: None,
            corrective_payment_required: None,
        }
    }

    /// Builder: attach a settle response.
    #[must_use]
    pub fn with_settle_response(mut self, settle: SettleResponse) -> Self {
        self.settle_response = Some(settle);
        self
    }

    /// Builder: attach a corrective payment-required challenge.
    #[must_use]
    pub fn with_corrective_payment_required(mut self, required: PaymentRequired) -> Self {
        self.corrective_payment_required = Some(required);
        self
    }
}

/// Result of [`ClientHooks::on_payment_response`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PaymentResponseResult {
    /// When `true`, the transport should retry once with a freshly built payload.
    pub recovered: bool,
}

impl PaymentResponseResult {
    /// No recovery — continue with the response as-is.
    #[must_use]
    pub const fn continue_() -> Self {
        Self { recovered: false }
    }

    /// Signal one corrective retry.
    #[must_use]
    pub const fn recovered() -> Self {
        Self { recovered: true }
    }
}

/// Lifecycle hooks for the payment client (V2).
///
/// All methods default to no-ops. Override only what you need.
pub trait ClientHooks: Send + Sync {
    /// Runs before payment payload creation. Abort skips signing.
    fn before_payment_creation<'a>(
        &'a self,
        _ctx: &'a PaymentCreationContext,
    ) -> impl Future<Output = HookDecision> + Send + 'a {
        async { HookDecision::Continue }
    }

    /// Runs after a payment payload is successfully created.
    fn after_payment_creation<'a>(
        &'a self,
        _ctx: &'a PaymentCreationContext,
        _created: &'a CreatedPayment,
    ) -> impl Future<Output = ()> + Send + 'a {
        async {}
    }

    /// Runs when payment creation fails. May recover with a substitute payload.
    fn on_payment_creation_failure<'a>(
        &'a self,
        _ctx: &'a PaymentCreationContext,
        _error: &'a str,
    ) -> impl Future<Output = FailureRecovery<CreatedPayment>> + Send + 'a {
        async { FailureRecovery::Propagate }
    }

    /// Runs after each paid response (settle success or corrective 402).
    ///
    /// Returning [`PaymentResponseResult::recovered`] asks the transport to
    /// retry once with a freshly built payment payload.
    fn on_payment_response<'a>(
        &'a self,
        _ctx: &'a PaymentResponseContext,
    ) -> impl Future<Output = PaymentResponseResult> + Send + 'a {
        async { PaymentResponseResult::continue_() }
    }
}

/// Object-safe erasure of [`ClientHooks`].
pub trait DynClientHooks: Send + Sync {
    /// See [`ClientHooks::before_payment_creation`].
    fn before_payment_creation<'a>(
        &'a self,
        ctx: &'a PaymentCreationContext,
    ) -> Pin<Box<dyn Future<Output = HookDecision> + Send + 'a>>;

    /// See [`ClientHooks::after_payment_creation`].
    fn after_payment_creation<'a>(
        &'a self,
        ctx: &'a PaymentCreationContext,
        created: &'a CreatedPayment,
    ) -> BoxFuture<'a, ()>;

    /// See [`ClientHooks::on_payment_creation_failure`].
    fn on_payment_creation_failure<'a>(
        &'a self,
        ctx: &'a PaymentCreationContext,
        error: &'a str,
    ) -> BoxFuture<'a, FailureRecovery<CreatedPayment>>;

    /// See [`ClientHooks::on_payment_response`].
    fn on_payment_response<'a>(
        &'a self,
        ctx: &'a PaymentResponseContext,
    ) -> Pin<Box<dyn Future<Output = PaymentResponseResult> + Send + 'a>>;
}

impl<T: ClientHooks + ?Sized> DynClientHooks for T {
    fn before_payment_creation<'a>(
        &'a self,
        ctx: &'a PaymentCreationContext,
    ) -> Pin<Box<dyn Future<Output = HookDecision> + Send + 'a>> {
        Box::pin(<Self as ClientHooks>::before_payment_creation(self, ctx))
    }

    fn after_payment_creation<'a>(
        &'a self,
        ctx: &'a PaymentCreationContext,
        created: &'a CreatedPayment,
    ) -> BoxFuture<'a, ()> {
        Box::pin(<Self as ClientHooks>::after_payment_creation(
            self, ctx, created,
        ))
    }

    fn on_payment_creation_failure<'a>(
        &'a self,
        ctx: &'a PaymentCreationContext,
        error: &'a str,
    ) -> BoxFuture<'a, FailureRecovery<CreatedPayment>> {
        Box::pin(<Self as ClientHooks>::on_payment_creation_failure(
            self, ctx, error,
        ))
    }

    fn on_payment_response<'a>(
        &'a self,
        ctx: &'a PaymentResponseContext,
    ) -> Pin<Box<dyn Future<Output = PaymentResponseResult> + Send + 'a>> {
        Box::pin(<Self as ClientHooks>::on_payment_response(self, ctx))
    }
}

impl Debug for dyn DynClientHooks {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("DynClientHooks")
    }
}

/// V2 payment client — scheme registry + selector + policies + hooks.
///
/// Mirrors Go/TS `x402Client` without V1 registration paths.
pub struct PaymentClient<S = FirstMatch> {
    schemes: Vec<Arc<dyn SchemeClient>>,
    selector: S,
    policies: Vec<Arc<dyn PaymentPolicy>>,
    hooks: Vec<Arc<dyn DynClientHooks>>,
}

impl PaymentClient<FirstMatch> {
    /// Empty client with [`FirstMatch`] selection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for PaymentClient<FirstMatch> {
    fn default() -> Self {
        Self {
            schemes: Vec::new(),
            selector: FirstMatch,
            policies: Vec::new(),
            hooks: Vec::new(),
        }
    }
}

impl<S> Debug for PaymentClient<S> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaymentClient")
            .field("schemes", &self.schemes.len())
            .field("policies", &self.policies.len())
            .field("hooks", &self.hooks.len())
            .finish_non_exhaustive()
    }
}

impl<S> PaymentClient<S> {
    /// Registers a V2 scheme client.
    #[must_use]
    pub fn register(mut self, scheme: impl SchemeClient + 'static) -> Self {
        self.schemes.push(Arc::new(scheme));
        self
    }

    /// Replaces the payment selector.
    #[must_use]
    pub fn with_selector<P: PaymentSelector>(self, selector: P) -> PaymentClient<P> {
        PaymentClient {
            schemes: self.schemes,
            selector,
            policies: self.policies,
            hooks: self.hooks,
        }
    }

    /// Appends a payment policy (applied in registration order).
    #[must_use]
    pub fn with_policy(mut self, policy: impl PaymentPolicy + 'static) -> Self {
        self.policies.push(Arc::new(policy));
        self
    }

    /// Registers a client lifecycle hook.
    #[must_use]
    pub fn with_hook(mut self, hook: impl ClientHooks + 'static) -> Self {
        self.hooks.push(Arc::new(hook));
        self
    }

    /// Number of registered scheme clients.
    #[must_use]
    pub fn scheme_count(&self) -> usize {
        self.schemes.len()
    }

    /// Number of registered hooks.
    #[must_use]
    pub fn hook_count(&self) -> usize {
        self.hooks.len()
    }
}

impl<S: PaymentSelector> PaymentClient<S> {
    /// Collects candidates from every registered scheme client.
    #[must_use]
    pub fn candidates(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        let mut out = Vec::new();
        for client in &self.schemes {
            out.extend(client.accept(payment_required));
        }
        out
    }

    /// Creates a signed payment payload for a 402 challenge.
    ///
    /// Runs before/after/failure hooks around selection + signing.
    ///
    /// # Errors
    ///
    /// Propagates selection, signing, and before-hook abort errors.
    pub async fn create_payment(
        &self,
        payment_required: &PaymentRequired,
    ) -> Result<CreatedPayment, ClientError> {
        let ctx = PaymentCreationContext {
            payment_required: payment_required.clone(),
        };

        if let Some(err) = self.run_before_creation(&ctx).await {
            return Err(err);
        }

        match self.create_payment_inner(payment_required).await {
            Ok(created) => {
                self.run_after_creation(&ctx, &created).await;
                Ok(created)
            }
            Err(err) => self.recover_creation(&ctx, err).await,
        }
    }

    async fn run_before_creation(&self, ctx: &PaymentCreationContext) -> Option<ClientError> {
        for hook in &self.hooks {
            if let Some(err) =
                Self::client_error_from_abort(hook.before_payment_creation(ctx).await)
            {
                return Some(err);
            }
        }
        None
    }

    fn client_error_from_abort(decision: HookDecision) -> Option<ClientError> {
        let HookDecision::Abort { reason, message } = decision else {
            return None;
        };
        let detail = if message.is_empty() {
            reason
        } else {
            format!("{reason}: {message}")
        };
        Some(ClientError::Parse(detail))
    }

    async fn run_after_creation(&self, ctx: &PaymentCreationContext, created: &CreatedPayment) {
        for hook in &self.hooks {
            hook.after_payment_creation(ctx, created).await;
        }
    }

    async fn recover_creation(
        &self,
        ctx: &PaymentCreationContext,
        err: ClientError,
    ) -> Result<CreatedPayment, ClientError> {
        let msg = err.to_string();
        for hook in &self.hooks {
            let recovery = hook.on_payment_creation_failure(ctx, &msg).await;
            if let FailureRecovery::Recovered(created) = recovery {
                return Ok(created);
            }
        }
        Err(err)
    }

    async fn create_payment_inner(
        &self,
        payment_required: &PaymentRequired,
    ) -> Result<CreatedPayment, ClientError> {
        let candidates = self.candidates(payment_required);
        let mut filtered: Vec<&PaymentCandidate> = candidates.iter().collect();
        for policy in &self.policies {
            filtered = policy.apply(filtered);
            if filtered.is_empty() {
                return Err(ClientError::NoMatchingPaymentOption);
            }
        }
        let selected = self
            .selector
            .select(&filtered)
            .ok_or(ClientError::NoMatchingPaymentOption)?;
        let signed_payload = selected.sign().await?;
        Ok(CreatedPayment::new(
            signed_payload,
            payment_required.clone(),
        ))
    }

    /// Dispatches [`ClientHooks::on_payment_response`] for every hook.
    ///
    /// First `recovered: true` wins; remaining hooks still run (instrumentation).
    pub async fn handle_payment_response(
        &self,
        ctx: &PaymentResponseContext,
    ) -> PaymentResponseResult {
        let mut recovered = false;
        for hook in &self.hooks {
            let result = hook.on_payment_response(ctx).await;
            if result.recovered {
                recovered = true;
            }
        }
        if recovered {
            PaymentResponseResult::recovered()
        } else {
            PaymentResponseResult::continue_()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;

    use super::*;
    use crate::chain::ChainId;
    use crate::scheme::{PaymentCandidateSigner, SchemeId, Sealed};
    use crate::wire::{PaymentRequirements, ResourceInfo};

    struct StubSigner(String);
    impl PaymentCandidateSigner for StubSigner {
        fn sign_payment<'a>(
            &'a self,
        ) -> Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + 'a>> {
            Box::pin(async move { Ok(self.0.clone()) })
        }
    }

    struct StubScheme;
    impl Sealed for StubScheme {}
    impl SchemeId for StubScheme {
        fn namespace(&self) -> &'static str {
            "eip155"
        }
        fn scheme(&self) -> &'static str {
            "exact"
        }
    }
    impl SchemeClient for StubScheme {
        fn accept(&self, required: &PaymentRequired) -> Vec<PaymentCandidate> {
            required
                .accepts
                .iter()
                .map(|r| PaymentCandidate {
                    chain_id: r.network.clone(),
                    asset: r.asset.clone(),
                    amount: r.amount.clone(),
                    scheme: r.scheme.clone(),
                    pay_to: r.pay_to.clone(),
                    signer: Box::new(StubSigner("c2lnbmVk".into())),
                })
                .collect()
        }
    }

    struct AbortHook;
    impl ClientHooks for AbortHook {
        fn before_payment_creation<'a>(
            &'a self,
            _: &PaymentCreationContext,
        ) -> impl Future<Output = HookDecision> + Send + 'a {
            std::future::ready(HookDecision::Abort {
                reason: "nope".into(),
                message: String::new(),
            })
        }
    }

    struct RecoverHook;
    impl ClientHooks for RecoverHook {
        fn on_payment_response<'a>(
            &'a self,
            _: &PaymentResponseContext,
        ) -> impl Future<Output = PaymentResponseResult> + Send + 'a {
            std::future::ready(PaymentResponseResult::recovered())
        }
    }

    fn sample_required() -> PaymentRequired {
        let req = PaymentRequirements::new(
            "exact".into(),
            "eip155:1".parse::<ChainId>().unwrap(),
            "1".into(),
            "0xa".into(),
            "0xb".into(),
            60,
        );
        PaymentRequired::new(ResourceInfo::new("https://example.com")).with_accepts(vec![req])
    }

    #[tokio::test]
    async fn create_payment_signs() {
        let client = PaymentClient::new().register(StubScheme);
        let created = client.create_payment(&sample_required()).await.unwrap();
        assert_eq!(created.signed_payload, "c2lnbmVk");
    }

    #[tokio::test]
    async fn before_hook_aborts() {
        let client = PaymentClient::new()
            .register(StubScheme)
            .with_hook(AbortHook);
        let err = client.create_payment(&sample_required()).await.unwrap_err();
        assert!(matches!(err, ClientError::Parse(s) if s.contains("nope")));
    }

    #[tokio::test]
    async fn payment_response_hook_recovers() {
        let client = PaymentClient::new().with_hook(RecoverHook);
        let ctx = PaymentResponseContext::new(sample_required(), "x")
            .with_corrective_payment_required(sample_required());
        let result = client.handle_payment_response(&ctx).await;
        assert!(result.recovered);
    }

    #[tokio::test]
    async fn no_schemes_yields_no_match() {
        let client = PaymentClient::new();
        let err = client.create_payment(&sample_required()).await.unwrap_err();
        assert!(matches!(err, ClientError::NoMatchingPaymentOption));
    }
}
