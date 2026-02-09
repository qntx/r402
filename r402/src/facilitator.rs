//! x402 facilitator base logic.
//!
//! Contains shared logic for the async-first x402 facilitator, including scheme
//! registration, routing, hook execution, and supported-kinds aggregation.
//!
//! Corresponds to Python SDK's `facilitator_base.py` + `facilitator.py`.

use std::collections::{HashMap, HashSet};

use crate::proto::helpers::matches_network_pattern;
use crate::proto::{
    Network, PaymentPayload, PaymentPayloadV1, PaymentRequirements, PaymentRequirementsV1,
    SettleResponse, SupportedKind, SupportedResponse, VerifyResponse,
};

use crate::error::{PaymentAbortedError, SchemeNotFoundError};
use crate::hooks::{
    AbortResult, PayloadView, RecoveredSettleResult, RecoveredVerifyResult, RequirementsView,
    SettleContext, SettleFailureContext, SettleResultContext, VerifyContext, VerifyFailureContext,
    VerifyResultContext,
};
use crate::scheme::{BoxFuture, SchemeError, SchemeFacilitator, SchemeFacilitatorV1};

/// Async hook called before verification. Return `Some(AbortResult)` to abort.
pub type BeforeVerifyHook =
    Box<dyn Fn(&VerifyContext) -> BoxFuture<'_, Option<AbortResult>> + Send + Sync>;

/// Async hook called after successful verification.
pub type AfterVerifyHook = Box<dyn Fn(&VerifyResultContext) -> BoxFuture<'_, ()> + Send + Sync>;

/// Async hook called on verification failure. Return recovery result to override.
pub type OnVerifyFailureHook = Box<
    dyn Fn(&VerifyFailureContext) -> BoxFuture<'_, Option<RecoveredVerifyResult>> + Send + Sync,
>;

/// Async hook called before settlement. Return `Some(AbortResult)` to abort.
pub type BeforeSettleHook =
    Box<dyn Fn(&SettleContext) -> BoxFuture<'_, Option<AbortResult>> + Send + Sync>;

/// Async hook called after successful settlement.
pub type AfterSettleHook = Box<dyn Fn(&SettleResultContext) -> BoxFuture<'_, ()> + Send + Sync>;

/// Async hook called on settlement failure. Return recovery result to override.
pub type OnSettleFailureHook = Box<
    dyn Fn(&SettleFailureContext) -> BoxFuture<'_, Option<RecoveredSettleResult>> + Send + Sync,
>;

/// Internal storage for a registered scheme facilitator.
struct SchemeData<T: ?Sized> {
    facilitator: Box<T>,
    networks: HashSet<Network>,
    pattern: Network,
}

/// Derives a common CAIP pattern from a set of networks.
///
/// If all networks share the same namespace, returns a wildcard pattern.
fn derive_pattern(networks: &HashSet<Network>) -> Network {
    let namespaces: HashSet<&str> = networks
        .iter()
        .filter_map(|n| n.split(':').next())
        .collect();
    if namespaces.len() == 1 {
        let ns = namespaces.into_iter().next().expect("non-empty set");
        format!("{ns}:*")
    } else {
        networks.iter().next().cloned().unwrap_or_default()
    }
}

/// Async-first x402 facilitator with scheme registration, routing, hooks,
/// and supported-kinds aggregation.
///
/// Corresponds to Python SDK's `x402Facilitator`.
pub struct X402Facilitator {
    schemes_v2: Vec<SchemeData<dyn SchemeFacilitator>>,
    schemes_v1: Vec<SchemeData<dyn SchemeFacilitatorV1>>,
    extensions: Vec<String>,
    before_verify_hooks: Vec<BeforeVerifyHook>,
    after_verify_hooks: Vec<AfterVerifyHook>,
    on_verify_failure_hooks: Vec<OnVerifyFailureHook>,
    before_settle_hooks: Vec<BeforeSettleHook>,
    after_settle_hooks: Vec<AfterSettleHook>,
    on_settle_failure_hooks: Vec<OnSettleFailureHook>,
}

impl std::fmt::Debug for X402Facilitator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X402Facilitator")
            .field("schemes_v2_count", &self.schemes_v2.len())
            .field("schemes_v1_count", &self.schemes_v1.len())
            .field("extensions", &self.extensions)
            .finish_non_exhaustive()
    }
}

impl Default for X402Facilitator {
    fn default() -> Self {
        Self::new()
    }
}

impl X402Facilitator {
    /// Creates a new facilitator.
    #[must_use]
    pub fn new() -> Self {
        Self {
            schemes_v2: Vec::new(),
            schemes_v1: Vec::new(),
            extensions: Vec::new(),
            before_verify_hooks: Vec::new(),
            after_verify_hooks: Vec::new(),
            on_verify_failure_hooks: Vec::new(),
            before_settle_hooks: Vec::new(),
            after_settle_hooks: Vec::new(),
            on_settle_failure_hooks: Vec::new(),
        }
    }

    /// Registers a V2 facilitator for one or more networks.
    pub fn register(
        &mut self,
        networks: Vec<Network>,
        facilitator: Box<dyn SchemeFacilitator>,
    ) -> &mut Self {
        let net_set: HashSet<Network> = networks.into_iter().collect();
        let pattern = derive_pattern(&net_set);
        self.schemes_v2.push(SchemeData {
            facilitator,
            networks: net_set,
            pattern,
        });
        self
    }

    /// Registers a V1 facilitator for one or more networks.
    pub fn register_v1(
        &mut self,
        networks: Vec<Network>,
        facilitator: Box<dyn SchemeFacilitatorV1>,
    ) -> &mut Self {
        let net_set: HashSet<Network> = networks.into_iter().collect();
        let pattern = derive_pattern(&net_set);
        self.schemes_v1.push(SchemeData {
            facilitator,
            networks: net_set,
            pattern,
        });
        self
    }

    /// Registers an extension name (e.g., `"bazaar"`).
    pub fn register_extension(&mut self, extension: String) -> &mut Self {
        if !self.extensions.contains(&extension) {
            self.extensions.push(extension);
        }
        self
    }

    /// Returns the list of registered extension names.
    #[must_use]
    pub fn extensions(&self) -> &[String] {
        &self.extensions
    }

    /// Registers a before-verify hook.
    pub fn on_before_verify(&mut self, hook: BeforeVerifyHook) -> &mut Self {
        self.before_verify_hooks.push(hook);
        self
    }

    /// Registers an after-verify hook.
    pub fn on_after_verify(&mut self, hook: AfterVerifyHook) -> &mut Self {
        self.after_verify_hooks.push(hook);
        self
    }

    /// Registers a verify-failure hook.
    pub fn on_verify_failure(&mut self, hook: OnVerifyFailureHook) -> &mut Self {
        self.on_verify_failure_hooks.push(hook);
        self
    }

    /// Registers a before-settle hook.
    pub fn on_before_settle(&mut self, hook: BeforeSettleHook) -> &mut Self {
        self.before_settle_hooks.push(hook);
        self
    }

    /// Registers an after-settle hook.
    pub fn on_after_settle(&mut self, hook: AfterSettleHook) -> &mut Self {
        self.after_settle_hooks.push(hook);
        self
    }

    /// Registers a settle-failure hook.
    pub fn on_settle_failure(&mut self, hook: OnSettleFailureHook) -> &mut Self {
        self.on_settle_failure_hooks.push(hook);
        self
    }

    /// Aggregates supported payment kinds and signers from all registered
    /// scheme facilitators.
    #[must_use]
    pub fn get_supported(&self) -> SupportedResponse {
        let mut kinds = Vec::new();
        let mut signers = HashMap::<String, Vec<String>>::new();

        for data in &self.schemes_v2 {
            let fac = &data.facilitator;
            for network in &data.networks {
                kinds.push(SupportedKind {
                    x402_version: 2,
                    scheme: fac.scheme().to_owned(),
                    network: network.clone(),
                    extra: fac.get_extra(network),
                });

                let family = fac.caip_family().to_owned();
                let network_signers = fac.get_signers(network);
                let entry = signers.entry(family).or_default();
                for s in network_signers {
                    if !entry.contains(&s) {
                        entry.push(s);
                    }
                }
            }
        }

        for data in &self.schemes_v1 {
            let fac = &data.facilitator;
            for network in &data.networks {
                kinds.push(SupportedKind {
                    x402_version: 1,
                    scheme: fac.scheme().to_owned(),
                    network: network.clone(),
                    extra: fac.get_extra(network),
                });

                let family = fac.caip_family().to_owned();
                let network_signers = fac.get_signers(network);
                let entry = signers.entry(family).or_default();
                for s in network_signers {
                    if !entry.contains(&s) {
                        entry.push(s);
                    }
                }
            }
        }

        SupportedResponse::new(kinds, self.extensions.clone(), signers)
    }

    /// Verifies a V2 payment with full hook lifecycle.
    ///
    /// # Errors
    ///
    /// Returns an error if no facilitator is registered or a hook aborts.
    pub async fn verify(
        &self,
        payload: &PaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<VerifyResponse, SchemeError> {
        let ctx = VerifyContext {
            payment_payload: PayloadView::V2(Box::new(payload.clone())),
            requirements: RequirementsView::V2(requirements.clone()),
            payload_bytes: None,
            requirements_bytes: None,
        };

        for hook in &self.before_verify_hooks {
            if let Some(abort) = hook(&ctx).await {
                return Err(Box::new(PaymentAbortedError::new(abort.reason)));
            }
        }

        let result = self.do_verify_v2(payload, requirements).await;

        match result {
            Ok(ref response) if response.is_valid => {
                let result_ctx = VerifyResultContext {
                    payment_payload: PayloadView::V2(Box::new(payload.clone())),
                    requirements: RequirementsView::V2(requirements.clone()),
                    payload_bytes: None,
                    requirements_bytes: None,
                    result: response.clone(),
                };
                for hook in &self.after_verify_hooks {
                    hook(&result_ctx).await;
                }
                result
            }
            Ok(ref response) => {
                let failure_ctx = VerifyFailureContext {
                    payment_payload: PayloadView::V2(Box::new(payload.clone())),
                    requirements: RequirementsView::V2(requirements.clone()),
                    payload_bytes: None,
                    requirements_bytes: None,
                    error: response.invalid_reason.clone().unwrap_or_default(),
                };
                for hook in &self.on_verify_failure_hooks {
                    if let Some(recovered) = hook(&failure_ctx).await {
                        return Ok(recovered.result);
                    }
                }
                result
            }
            Err(e) => {
                let failure_ctx = VerifyFailureContext {
                    payment_payload: PayloadView::V2(Box::new(payload.clone())),
                    requirements: RequirementsView::V2(requirements.clone()),
                    payload_bytes: None,
                    requirements_bytes: None,
                    error: e.to_string(),
                };
                for hook in &self.on_verify_failure_hooks {
                    if let Some(recovered) = hook(&failure_ctx).await {
                        return Ok(recovered.result);
                    }
                }
                Err(e)
            }
        }
    }

    /// Settles a V2 payment with full hook lifecycle.
    ///
    /// # Errors
    ///
    /// Returns an error if no facilitator is registered or a hook aborts.
    pub async fn settle(
        &self,
        payload: &PaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<SettleResponse, SchemeError> {
        let ctx = SettleContext {
            payment_payload: PayloadView::V2(Box::new(payload.clone())),
            requirements: RequirementsView::V2(requirements.clone()),
            payload_bytes: None,
            requirements_bytes: None,
        };

        for hook in &self.before_settle_hooks {
            if let Some(abort) = hook(&ctx).await {
                return Err(Box::new(PaymentAbortedError::new(abort.reason)));
            }
        }

        let result = self.do_settle_v2(payload, requirements).await;

        match result {
            Ok(ref response) if response.success => {
                let result_ctx = SettleResultContext {
                    payment_payload: PayloadView::V2(Box::new(payload.clone())),
                    requirements: RequirementsView::V2(requirements.clone()),
                    payload_bytes: None,
                    requirements_bytes: None,
                    result: response.clone(),
                };
                for hook in &self.after_settle_hooks {
                    hook(&result_ctx).await;
                }
                result
            }
            Ok(ref response) => {
                let failure_ctx = SettleFailureContext {
                    payment_payload: PayloadView::V2(Box::new(payload.clone())),
                    requirements: RequirementsView::V2(requirements.clone()),
                    payload_bytes: None,
                    requirements_bytes: None,
                    error: response.error_reason.clone().unwrap_or_default(),
                };
                for hook in &self.on_settle_failure_hooks {
                    if let Some(recovered) = hook(&failure_ctx).await {
                        return Ok(recovered.result);
                    }
                }
                result
            }
            Err(e) => {
                let failure_ctx = SettleFailureContext {
                    payment_payload: PayloadView::V2(Box::new(payload.clone())),
                    requirements: RequirementsView::V2(requirements.clone()),
                    payload_bytes: None,
                    requirements_bytes: None,
                    error: e.to_string(),
                };
                for hook in &self.on_settle_failure_hooks {
                    if let Some(recovered) = hook(&failure_ctx).await {
                        return Ok(recovered.result);
                    }
                }
                Err(e)
            }
        }
    }

    /// Verifies a V1 payment with full hook lifecycle.
    ///
    /// # Errors
    ///
    /// Returns an error if no facilitator is registered or a hook aborts.
    pub async fn verify_v1(
        &self,
        payload: &PaymentPayloadV1,
        requirements: &PaymentRequirementsV1,
    ) -> Result<VerifyResponse, SchemeError> {
        let ctx = VerifyContext {
            payment_payload: PayloadView::V1(payload.clone()),
            requirements: RequirementsView::V1(requirements.clone()),
            payload_bytes: None,
            requirements_bytes: None,
        };

        for hook in &self.before_verify_hooks {
            if let Some(abort) = hook(&ctx).await {
                return Err(Box::new(PaymentAbortedError::new(abort.reason)));
            }
        }

        let result = self.do_verify_v1(payload, requirements).await;

        match result {
            Ok(ref response) if response.is_valid => {
                let result_ctx = VerifyResultContext {
                    payment_payload: PayloadView::V1(payload.clone()),
                    requirements: RequirementsView::V1(requirements.clone()),
                    payload_bytes: None,
                    requirements_bytes: None,
                    result: response.clone(),
                };
                for hook in &self.after_verify_hooks {
                    hook(&result_ctx).await;
                }
                result
            }
            Ok(ref response) => {
                let failure_ctx = VerifyFailureContext {
                    payment_payload: PayloadView::V1(payload.clone()),
                    requirements: RequirementsView::V1(requirements.clone()),
                    payload_bytes: None,
                    requirements_bytes: None,
                    error: response.invalid_reason.clone().unwrap_or_default(),
                };
                for hook in &self.on_verify_failure_hooks {
                    if let Some(recovered) = hook(&failure_ctx).await {
                        return Ok(recovered.result);
                    }
                }
                result
            }
            Err(e) => {
                let failure_ctx = VerifyFailureContext {
                    payment_payload: PayloadView::V1(payload.clone()),
                    requirements: RequirementsView::V1(requirements.clone()),
                    payload_bytes: None,
                    requirements_bytes: None,
                    error: e.to_string(),
                };
                for hook in &self.on_verify_failure_hooks {
                    if let Some(recovered) = hook(&failure_ctx).await {
                        return Ok(recovered.result);
                    }
                }
                Err(e)
            }
        }
    }

    /// Settles a V1 payment with full hook lifecycle.
    ///
    /// # Errors
    ///
    /// Returns an error if no facilitator is registered or a hook aborts.
    pub async fn settle_v1(
        &self,
        payload: &PaymentPayloadV1,
        requirements: &PaymentRequirementsV1,
    ) -> Result<SettleResponse, SchemeError> {
        let ctx = SettleContext {
            payment_payload: PayloadView::V1(payload.clone()),
            requirements: RequirementsView::V1(requirements.clone()),
            payload_bytes: None,
            requirements_bytes: None,
        };

        for hook in &self.before_settle_hooks {
            if let Some(abort) = hook(&ctx).await {
                return Err(Box::new(PaymentAbortedError::new(abort.reason)));
            }
        }

        let result = self.do_settle_v1(payload, requirements).await;

        match result {
            Ok(ref response) if response.success => {
                let result_ctx = SettleResultContext {
                    payment_payload: PayloadView::V1(payload.clone()),
                    requirements: RequirementsView::V1(requirements.clone()),
                    payload_bytes: None,
                    requirements_bytes: None,
                    result: response.clone(),
                };
                for hook in &self.after_settle_hooks {
                    hook(&result_ctx).await;
                }
                result
            }
            Ok(ref response) => {
                let failure_ctx = SettleFailureContext {
                    payment_payload: PayloadView::V1(payload.clone()),
                    requirements: RequirementsView::V1(requirements.clone()),
                    payload_bytes: None,
                    requirements_bytes: None,
                    error: response.error_reason.clone().unwrap_or_default(),
                };
                for hook in &self.on_settle_failure_hooks {
                    if let Some(recovered) = hook(&failure_ctx).await {
                        return Ok(recovered.result);
                    }
                }
                result
            }
            Err(e) => {
                let failure_ctx = SettleFailureContext {
                    payment_payload: PayloadView::V1(payload.clone()),
                    requirements: RequirementsView::V1(requirements.clone()),
                    payload_bytes: None,
                    requirements_bytes: None,
                    error: e.to_string(),
                };
                for hook in &self.on_settle_failure_hooks {
                    if let Some(recovered) = hook(&failure_ctx).await {
                        return Ok(recovered.result);
                    }
                }
                Err(e)
            }
        }
    }

    /// Finds the V2 facilitator for a given scheme and network.
    fn find_facilitator(&self, scheme: &str, network: &str) -> Option<&dyn SchemeFacilitator> {
        for data in &self.schemes_v2 {
            if data.facilitator.scheme() != scheme {
                continue;
            }
            if data.networks.contains(network) {
                return Some(&*data.facilitator);
            }
            if matches_network_pattern(network, &data.pattern) {
                return Some(&*data.facilitator);
            }
        }
        None
    }

    /// Finds the V1 facilitator for a given scheme and network.
    fn find_facilitator_v1(&self, scheme: &str, network: &str) -> Option<&dyn SchemeFacilitatorV1> {
        for data in &self.schemes_v1 {
            if data.facilitator.scheme() != scheme {
                continue;
            }
            if data.networks.contains(network) {
                return Some(&*data.facilitator);
            }
            if matches_network_pattern(network, &data.pattern) {
                return Some(&*data.facilitator);
            }
        }
        None
    }

    /// Inner V1 verify (no hooks).
    async fn do_verify_v1(
        &self,
        payload: &PaymentPayloadV1,
        requirements: &PaymentRequirementsV1,
    ) -> Result<VerifyResponse, SchemeError> {
        let scheme = payload.scheme();
        let network = payload.network();
        let fac = self
            .find_facilitator_v1(scheme, network)
            .ok_or_else(|| SchemeNotFoundError::new(scheme, network))?;
        Ok(fac.verify(payload, requirements).await)
    }

    /// Inner V1 settle (no hooks).
    async fn do_settle_v1(
        &self,
        payload: &PaymentPayloadV1,
        requirements: &PaymentRequirementsV1,
    ) -> Result<SettleResponse, SchemeError> {
        let scheme = payload.scheme();
        let network = payload.network();
        let fac = self
            .find_facilitator_v1(scheme, network)
            .ok_or_else(|| SchemeNotFoundError::new(scheme, network))?;
        Ok(fac.settle(payload, requirements).await)
    }

    /// Inner V2 verify (no hooks).
    async fn do_verify_v2(
        &self,
        payload: &PaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<VerifyResponse, SchemeError> {
        let scheme = payload.scheme();
        let network = payload.network();
        let fac = self
            .find_facilitator(scheme, network)
            .ok_or_else(|| SchemeNotFoundError::new(scheme, network))?;
        Ok(fac.verify(payload, requirements).await)
    }

    /// Inner V2 settle (no hooks).
    async fn do_settle_v2(
        &self,
        payload: &PaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<SettleResponse, SchemeError> {
        let scheme = payload.scheme();
        let network = payload.network();
        let fac = self
            .find_facilitator(scheme, network)
            .ok_or_else(|| SchemeNotFoundError::new(scheme, network))?;
        Ok(fac.settle(payload, requirements).await)
    }
}
