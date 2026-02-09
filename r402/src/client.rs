//! x402 client base logic.
//!
//! Contains shared logic for the async-first x402 client, including scheme
//! registration, requirement selection policies, hook execution, and
//! payment creation.
//!
//! Corresponds to Python SDK's `client_base.py` + `client.py`.

use std::collections::HashMap;

use crate::proto::helpers::find_schemes_by_network;
use crate::proto::{
    Network, PaymentPayload, PaymentPayloadV1, PaymentRequired, PaymentRequiredV1,
    PaymentRequirements, PaymentRequirementsV1,
};

use crate::error::{NoMatchingRequirementsError, PaymentAbortedError, SchemeNotFoundError};
use crate::hooks::{
    AbortResult, PayloadView, PaymentCreatedContext, PaymentCreationContext,
    PaymentCreationFailureContext, PaymentRequiredView, RecoveredPayloadResult, RequirementsView,
};
use crate::scheme::{BoxFuture, SchemeClient, SchemeClientV1, SchemeError};

/// Policy function that filters and reorders requirements.
///
/// Takes the protocol version and a list of requirements, returns a
/// filtered/reordered list. Corresponds to Python SDK's `PaymentPolicy`.
pub type PaymentPolicy =
    Box<dyn Fn(u32, Vec<RequirementsView>) -> Vec<RequirementsView> + Send + Sync>;

/// Selector function that picks the final requirement from a filtered list.
///
/// Corresponds to Python SDK's `PaymentRequirementsSelector`.
pub type PaymentRequirementsSelector = Box<dyn Fn(u32, &[RequirementsView]) -> usize + Send + Sync>;

/// Async hook called before payment creation. Return `Some(AbortResult)` to abort.
pub type BeforePaymentCreationHook =
    Box<dyn Fn(&PaymentCreationContext) -> BoxFuture<'_, Option<AbortResult>> + Send + Sync>;

/// Async hook called after successful payment creation.
pub type AfterPaymentCreationHook =
    Box<dyn Fn(&PaymentCreatedContext) -> BoxFuture<'_, ()> + Send + Sync>;

/// Async hook called on payment creation failure. Return `Some(RecoveredPayloadResult)` to recover.
pub type OnPaymentCreationFailureHook = Box<
    dyn Fn(&PaymentCreationFailureContext) -> BoxFuture<'_, Option<RecoveredPayloadResult>>
        + Send
        + Sync,
>;

/// Creates a policy that prefers a specific network.
///
/// Requirements matching the given network are placed first.
#[must_use]
pub fn prefer_network(network: Network) -> PaymentPolicy {
    Box::new(move |_version, reqs| {
        let mut preferred = Vec::new();
        let mut others = Vec::new();
        for r in reqs {
            if r.network() == network {
                preferred.push(r);
            } else {
                others.push(r);
            }
        }
        preferred.extend(others);
        preferred
    })
}

/// Creates a policy that prefers a specific scheme.
///
/// Requirements matching the given scheme are placed first.
#[must_use]
pub fn prefer_scheme(scheme: String) -> PaymentPolicy {
    Box::new(move |_version, reqs| {
        let mut preferred = Vec::new();
        let mut others = Vec::new();
        for r in reqs {
            if r.scheme() == scheme {
                preferred.push(r);
            } else {
                others.push(r);
            }
        }
        preferred.extend(others);
        preferred
    })
}

/// Creates a policy that filters by maximum amount.
///
/// Only requirements with `amount <= max_value` are kept.
#[must_use]
pub fn max_amount(max_value: u128) -> PaymentPolicy {
    Box::new(move |_version, reqs| {
        reqs.into_iter()
            .filter(|r| r.amount().parse::<u128>().is_ok_and(|a| a <= max_value))
            .collect()
    })
}

/// Default selector: returns the first requirement.
const fn default_selector(_version: u32, _reqs: &[RequirementsView]) -> usize {
    0
}

/// Async-first x402 client with scheme registration, policies, hooks,
/// and payment creation.
///
/// Corresponds to Python SDK's `x402Client`.
pub struct X402Client {
    schemes_v2: HashMap<Network, HashMap<String, Box<dyn SchemeClient>>>,
    schemes_v1: HashMap<Network, HashMap<String, Box<dyn SchemeClientV1>>>,
    policies: Vec<PaymentPolicy>,
    selector: PaymentRequirementsSelector,
    before_hooks: Vec<BeforePaymentCreationHook>,
    after_hooks: Vec<AfterPaymentCreationHook>,
    failure_hooks: Vec<OnPaymentCreationFailureHook>,
}

impl std::fmt::Debug for X402Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X402Client")
            .field(
                "schemes_v2_networks",
                &self.schemes_v2.keys().collect::<Vec<_>>(),
            )
            .field(
                "schemes_v1_networks",
                &self.schemes_v1.keys().collect::<Vec<_>>(),
            )
            .field("policies_count", &self.policies.len())
            .field("before_hooks", &self.before_hooks.len())
            .field("after_hooks", &self.after_hooks.len())
            .field("failure_hooks", &self.failure_hooks.len())
            .finish_non_exhaustive()
    }
}

impl Default for X402Client {
    fn default() -> Self {
        Self::new()
    }
}

impl X402Client {
    /// Creates a new client with default selector.
    #[must_use]
    pub fn new() -> Self {
        Self {
            schemes_v2: HashMap::new(),
            schemes_v1: HashMap::new(),
            policies: Vec::new(),
            selector: Box::new(default_selector),
            before_hooks: Vec::new(),
            after_hooks: Vec::new(),
            failure_hooks: Vec::new(),
        }
    }

    /// Creates a new client with a custom selector.
    #[must_use]
    pub fn with_selector(selector: PaymentRequirementsSelector) -> Self {
        Self {
            selector,
            ..Self::new()
        }
    }

    /// Registers a V2 scheme client for a network.
    pub fn register(&mut self, network: Network, client: Box<dyn SchemeClient>) -> &mut Self {
        let scheme = client.scheme().to_owned();
        self.schemes_v2
            .entry(network)
            .or_default()
            .insert(scheme, client);
        self
    }

    /// Registers a V1 scheme client for a network.
    pub fn register_v1(&mut self, network: Network, client: Box<dyn SchemeClientV1>) -> &mut Self {
        let scheme = client.scheme().to_owned();
        self.schemes_v1
            .entry(network)
            .or_default()
            .insert(scheme, client);
        self
    }

    /// Adds a requirement filter policy.
    pub fn register_policy(&mut self, policy: PaymentPolicy) -> &mut Self {
        self.policies.push(policy);
        self
    }

    /// Registers a before-payment-creation hook. Return `AbortResult` to abort.
    pub fn on_before_payment_creation(&mut self, hook: BeforePaymentCreationHook) -> &mut Self {
        self.before_hooks.push(hook);
        self
    }

    /// Registers an after-payment-creation hook.
    pub fn on_after_payment_creation(&mut self, hook: AfterPaymentCreationHook) -> &mut Self {
        self.after_hooks.push(hook);
        self
    }

    /// Registers a failure hook. Return `RecoveredPayloadResult` to recover.
    pub fn on_payment_creation_failure(&mut self, hook: OnPaymentCreationFailureHook) -> &mut Self {
        self.failure_hooks.push(hook);
        self
    }

    /// Selects V2 requirements using policies and selector.
    fn select_requirements_v2(
        &self,
        requirements: &[PaymentRequirements],
    ) -> Result<PaymentRequirements, NoMatchingRequirementsError> {
        let supported: Vec<RequirementsView> = requirements
            .iter()
            .filter(|req| {
                find_schemes_by_network(&self.schemes_v2, &req.network)
                    .is_some_and(|schemes| schemes.contains_key(&req.scheme))
            })
            .cloned()
            .map(RequirementsView::V2)
            .collect();

        if supported.is_empty() {
            return Err(NoMatchingRequirementsError::new(
                "No payment requirements match registered schemes",
            ));
        }

        let mut filtered = supported;
        for policy in &self.policies {
            filtered = policy(2, filtered);
            if filtered.is_empty() {
                return Err(NoMatchingRequirementsError::new(
                    "All requirements filtered out by policies",
                ));
            }
        }

        let idx = (self.selector)(2, &filtered);
        match filtered.into_iter().nth(idx) {
            Some(RequirementsView::V2(r)) => Ok(r),
            _ => Err(NoMatchingRequirementsError::new(
                "Selector returned invalid index",
            )),
        }
    }

    /// Selects V1 requirements using policies and selector.
    fn select_requirements_v1(
        &self,
        requirements: &[PaymentRequirementsV1],
    ) -> Result<PaymentRequirementsV1, NoMatchingRequirementsError> {
        let supported: Vec<RequirementsView> = requirements
            .iter()
            .filter(|req| {
                find_schemes_by_network(&self.schemes_v1, &req.network)
                    .is_some_and(|schemes| schemes.contains_key(&req.scheme))
            })
            .cloned()
            .map(RequirementsView::V1)
            .collect();

        if supported.is_empty() {
            return Err(NoMatchingRequirementsError::new(
                "No payment requirements match registered schemes",
            ));
        }

        let mut filtered = supported;
        for policy in &self.policies {
            filtered = policy(1, filtered);
            if filtered.is_empty() {
                return Err(NoMatchingRequirementsError::new(
                    "All requirements filtered out by policies",
                ));
            }
        }

        let idx = (self.selector)(1, &filtered);
        match filtered.into_iter().nth(idx) {
            Some(RequirementsView::V1(r)) => Ok(r),
            _ => Err(NoMatchingRequirementsError::new(
                "Selector returned invalid index",
            )),
        }
    }

    /// Creates a payment payload from a 402 response.
    ///
    /// Automatically routes to V1 or V2 based on the `x402_version` field.
    ///
    /// # Errors
    ///
    /// Returns an error if requirement selection, hook execution, or
    /// payload creation fails.
    pub async fn create_payment_payload(
        &self,
        payment_required: &PaymentRequired,
    ) -> Result<PaymentPayload, SchemeError> {
        self.create_payment_payload_v2(payment_required).await
    }

    /// Creates a V2 payment payload with full hook lifecycle.
    async fn create_payment_payload_v2(
        &self,
        payment_required: &PaymentRequired,
    ) -> Result<PaymentPayload, SchemeError> {
        let selected = self.select_requirements_v2(&payment_required.accepts)?;

        let context = PaymentCreationContext {
            payment_required: PaymentRequiredView::V2(payment_required.clone()),
            selected_requirements: RequirementsView::V2(selected.clone()),
        };

        for hook in &self.before_hooks {
            if let Some(abort) = hook(&context).await {
                return Err(Box::new(PaymentAbortedError::new(abort.reason)));
            }
        }

        let result = self.do_create_v2(payment_required, &selected).await;

        match result {
            Ok(payload) => {
                let created = PaymentCreatedContext {
                    payment_required: PaymentRequiredView::V2(payment_required.clone()),
                    selected_requirements: RequirementsView::V2(selected.clone()),
                    payment_payload: PayloadView::V2(Box::new(payload.clone())),
                };
                for hook in &self.after_hooks {
                    hook(&created).await;
                }
                Ok(payload)
            }
            Err(e) => {
                let failure = PaymentCreationFailureContext {
                    payment_required: PaymentRequiredView::V2(payment_required.clone()),
                    selected_requirements: RequirementsView::V2(selected),
                    error: e.to_string(),
                };
                for hook in &self.failure_hooks {
                    if let Some(RecoveredPayloadResult::V2(p)) = hook(&failure).await {
                        return Ok(*p);
                    }
                }
                Err(e)
            }
        }
    }

    /// Inner V2 payload creation (no hooks).
    async fn do_create_v2(
        &self,
        payment_required: &PaymentRequired,
        selected: &PaymentRequirements,
    ) -> Result<PaymentPayload, SchemeError> {
        let schemes = find_schemes_by_network(&self.schemes_v2, &selected.network)
            .ok_or_else(|| SchemeNotFoundError::new(&selected.scheme, &selected.network))?;

        let client = schemes
            .get(&selected.scheme)
            .ok_or_else(|| SchemeNotFoundError::new(&selected.scheme, &selected.network))?;

        let inner_payload = client.create_payment_payload(selected).await?;

        Ok(PaymentPayload {
            x402_version: 2,
            payload: inner_payload,
            resource: payment_required.resource.clone(),
            extensions: payment_required.extensions.clone(),
            accepted: selected.clone(),
        })
    }

    /// Creates a V1 payment payload with full hook lifecycle.
    ///
    /// # Errors
    ///
    /// Returns an error if requirement selection, hook execution, or
    /// payload creation fails.
    pub async fn create_payment_payload_v1(
        &self,
        payment_required: &PaymentRequiredV1,
    ) -> Result<PaymentPayloadV1, SchemeError> {
        let selected = self.select_requirements_v1(&payment_required.accepts)?;

        let context = PaymentCreationContext {
            payment_required: PaymentRequiredView::V1(payment_required.clone()),
            selected_requirements: RequirementsView::V1(selected.clone()),
        };

        for hook in &self.before_hooks {
            if let Some(abort) = hook(&context).await {
                return Err(Box::new(PaymentAbortedError::new(abort.reason)));
            }
        }

        let result = self.do_create_v1(payment_required, &selected).await;

        match result {
            Ok(payload) => {
                let created = PaymentCreatedContext {
                    payment_required: PaymentRequiredView::V1(payment_required.clone()),
                    selected_requirements: RequirementsView::V1(selected.clone()),
                    payment_payload: PayloadView::V1(payload.clone()),
                };
                for hook in &self.after_hooks {
                    hook(&created).await;
                }
                Ok(payload)
            }
            Err(e) => {
                let failure = PaymentCreationFailureContext {
                    payment_required: PaymentRequiredView::V1(payment_required.clone()),
                    selected_requirements: RequirementsView::V1(selected),
                    error: e.to_string(),
                };
                for hook in &self.failure_hooks {
                    if let Some(RecoveredPayloadResult::V1(p)) = hook(&failure).await {
                        return Ok(p);
                    }
                }
                Err(e)
            }
        }
    }

    /// Inner V1 payload creation (no hooks).
    async fn do_create_v1(
        &self,
        _payment_required: &PaymentRequiredV1,
        selected: &PaymentRequirementsV1,
    ) -> Result<PaymentPayloadV1, SchemeError> {
        let schemes = find_schemes_by_network(&self.schemes_v1, &selected.network)
            .ok_or_else(|| SchemeNotFoundError::new(&selected.scheme, &selected.network))?;

        let client = schemes
            .get(&selected.scheme)
            .ok_or_else(|| SchemeNotFoundError::new(&selected.scheme, &selected.network))?;

        let inner_payload = client.create_payment_payload(selected).await?;

        Ok(PaymentPayloadV1 {
            x402_version: 1,
            scheme: selected.scheme.clone(),
            network: selected.network.clone(),
            payload: inner_payload,
        })
    }

    /// Returns registered schemes for debugging.
    #[must_use]
    pub fn registered_schemes(&self) -> HashMap<u32, Vec<(String, String)>> {
        let mut result = HashMap::new();

        let mut v2 = Vec::new();
        for (network, schemes) in &self.schemes_v2 {
            for scheme in schemes.keys() {
                v2.push((network.clone(), scheme.clone()));
            }
        }
        result.insert(2, v2);

        let mut v1 = Vec::new();
        for (network, schemes) in &self.schemes_v1 {
            for scheme in schemes.keys() {
                v1.push((network.clone(), scheme.clone()));
            }
        }
        result.insert(1, v1);

        result
    }
}
