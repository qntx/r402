//! [`PaymentClient`]: register schemes, policies, hooks, and spend controls.

use std::fmt::{self, Debug, Formatter};
use std::sync::Arc;

use r402_protocol::{ClientError, PaymentRequired};
#[cfg(test)]
use serde_json as _;
#[cfg(test)]
use tokio as _;

use crate::candidate::SchemeClient;
use crate::hooks::{
    ClientHooks, CreatedPayment, DynClientHooks, FailureRecovery, HookDecision,
    PaymentCreationContext,
};
use crate::policy::PaymentPolicy;
use crate::select::{FirstMatch, PaymentSelector};
use crate::spend::SpendControls;

/// Buyer payment client.
pub struct PaymentClient<S = FirstMatch> {
    pub(crate) schemes: Vec<Arc<dyn SchemeClient>>,
    pub(crate) selector: S,
    pub(crate) policies: Vec<Arc<dyn PaymentPolicy>>,
    pub(crate) hooks: Vec<Arc<dyn DynClientHooks>>,
    pub(crate) spend_controls: Option<SpendControls>,
}

impl PaymentClient<FirstMatch> {
    /// Empty client with [`FirstMatch`] selection and default `$1` spend controls.
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
            spend_controls: Some(SpendControls::default()),
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
    /// Registers a scheme client.
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
            spend_controls: self.spend_controls,
        }
    }

    /// Enables spend controls with the given configuration.
    #[must_use]
    pub fn with_spend_controls(mut self, controls: SpendControls) -> Self {
        self.spend_controls = Some(controls);
        self
    }

    /// Disables all spend controls (any asset, no caps).
    #[must_use]
    pub fn disable_spend_controls(mut self) -> Self {
        self.spend_controls = None;
        self
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
    /// Creates a signed payment payload for a 402 challenge.
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
        let selected = self.select_candidate(&candidates)?;
        let signed_payload = selected.sign().await?;
        Ok(CreatedPayment::new(
            signed_payload,
            payment_required.clone(),
        ))
    }
}
