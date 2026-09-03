//! Resource-server orchestration: scheme table, hooks, and facilitator handle.

use std::sync::Arc;

use r402_facilitator::{DynFacilitator, Facilitator};
use r402_protocol::extension::{Extension, ExtensionRegistry};
use r402_protocol::network::{ChainId, ChainIdPattern};
use r402_protocol::payment::{PaymentRequirements, SupportedResponse, V2};

use crate::hooks::{
    CancelReason, DynResourceServerHooks, PaymentHookContext, ResourceServerHooks,
    VerifiedPaymentCanceledContext, WirePaymentPayload,
};
use crate::payment_flow::{
    PaymentFlowError, PaymentFlowName, PaymentFlowScheme, ResolvedPaymentFlow, SettlePhase,
    resolve_payment_flow,
};
use crate::scheme::{DynSchemeNetworkServer, FacilitatorSupportError, SchemeNetworkServer};

/// Server-side payment orchestrator.
pub struct ResourceServer {
    pub(crate) facilitator: Arc<dyn DynFacilitator>,
    pub(crate) hooks: Vec<Arc<dyn DynResourceServerHooks>>,
    pub(crate) schemes: Vec<(ChainIdPattern, Arc<dyn DynSchemeNetworkServer>)>,
    pub(crate) extensions: ExtensionRegistry,
}

impl Clone for ResourceServer {
    fn clone(&self) -> Self {
        Self {
            facilitator: Arc::clone(&self.facilitator),
            hooks: self.hooks.clone(),
            schemes: self.schemes.clone(),
            extensions: self.extensions.clone(),
        }
    }
}

impl std::fmt::Debug for ResourceServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceServer")
            .field("hooks", &self.hooks.len())
            .field("schemes", &self.schemes.len())
            .field("extensions", &self.extensions.len())
            .finish_non_exhaustive()
    }
}

impl ResourceServer {
    /// Creates a resource server over any [`Facilitator`].
    #[must_use]
    pub fn new<F>(facilitator: Arc<F>) -> Self
    where
        F: Facilitator + 'static,
    {
        let erased: Arc<dyn DynFacilitator> = facilitator;
        Self {
            facilitator: erased,
            hooks: Vec::new(),
            schemes: Vec::new(),
            extensions: ExtensionRegistry::new(),
        }
    }

    /// Creates from an already-erased facilitator handle.
    #[must_use]
    pub fn from_dyn(facilitator: Arc<dyn DynFacilitator>) -> Self {
        Self {
            facilitator,
            hooks: Vec::new(),
            schemes: Vec::new(),
            extensions: ExtensionRegistry::new(),
        }
    }

    /// Registers a lifecycle hook. Returns `self` for builder chaining.
    #[must_use]
    pub fn with_hook(mut self, hook: impl ResourceServerHooks + 'static) -> Self {
        self.hooks.push(Arc::new(hook));
        self
    }

    /// Registers a lifecycle hook after construction.
    pub fn add_hook(&mut self, hook: impl ResourceServerHooks + 'static) {
        self.hooks.push(Arc::new(hook));
    }

    /// Number of registered resource-server hooks.
    #[must_use]
    pub fn hook_count(&self) -> usize {
        self.hooks.len()
    }

    /// Returns a clone of the inner facilitator handle.
    #[must_use]
    pub fn facilitator(&self) -> Arc<dyn DynFacilitator> {
        Arc::clone(&self.facilitator)
    }

    /// Registers a scheme/network adapter. Returns `self` for builder chaining.
    #[must_use]
    pub fn with_scheme(
        mut self,
        network: ChainIdPattern,
        scheme: impl SchemeNetworkServer + 'static,
    ) -> Self {
        self.register_scheme(network, scheme);
        self
    }

    /// Registers a scheme/network adapter after construction.
    pub fn register_scheme(
        &mut self,
        network: ChainIdPattern,
        scheme: impl SchemeNetworkServer + 'static,
    ) {
        self.schemes.push((network, Arc::new(scheme)));
    }

    /// Registered adapter for `scheme` on `network`, if any.
    ///
    /// Exact CAIP-2 registrations win over wildcard/set patterns.
    #[must_use]
    pub fn registered_scheme(
        &self,
        scheme: &str,
        network: &ChainId,
    ) -> Option<&dyn DynSchemeNetworkServer> {
        let mut patterned: Option<&dyn DynSchemeNetworkServer> = None;
        for (pattern, server) in &self.schemes {
            if server.scheme() != scheme || !pattern.matches(network) {
                continue;
            }
            if matches!(pattern, ChainIdPattern::Exact { .. }) {
                return Some(server.as_ref());
            }
            if patterned.is_none() {
                patterned = Some(server.as_ref());
            }
        }
        patterned
    }

    /// Resolves ATM + payment flow from the registered scheme table.
    ///
    /// # Errors
    ///
    /// [`PaymentFlowError::UnregisteredScheme`] when no adapter matches this
    /// accept, or ATM/flow errors from [`resolve_payment_flow`].
    pub(crate) fn resolved_payment_flow(
        &self,
        requirements: &PaymentRequirements,
    ) -> Result<ResolvedPaymentFlow, PaymentFlowError> {
        let Some(scheme) =
            self.registered_scheme(requirements.scheme.as_str(), &requirements.network)
        else {
            return Err(PaymentFlowError::UnregisteredScheme {
                scheme: requirements.scheme.to_string(),
                network: requirements.network.to_string(),
            });
        };
        resolve_payment_flow(
            &PaymentFlowScheme {
                scheme: scheme.scheme(),
                default_asset_transfer_method: scheme.default_asset_transfer_method(),
                payment_flows: scheme.payment_flows(),
            },
            requirements,
        )
    }

    /// Resolves the payment-flow name for this accept from the scheme table.
    ///
    /// # Errors
    ///
    /// [`PaymentFlowError::UnregisteredScheme`] when no adapter matches this
    /// accept, or ATM/flow errors from [`resolve_payment_flow`].
    pub fn get_payment_flow(
        &self,
        requirements: &PaymentRequirements,
    ) -> Result<PaymentFlowName, PaymentFlowError> {
        Ok(self.resolved_payment_flow(requirements)?.payment_flow)
    }

    /// Whether the registered scheme yields cancel-settle requirements.
    ///
    /// Escrow accepts must return `true` before a transport will serve them.
    pub async fn has_settle_on_cancel(&self, requirements: &PaymentRequirements) -> bool {
        let Some(scheme) =
            self.registered_scheme(requirements.scheme.as_str(), &requirements.network)
        else {
            return false;
        };
        let ctx = VerifiedPaymentCanceledContext {
            payment: PaymentHookContext {
                payload: WirePaymentPayload::new(requirements.clone(), serde_json::Value::Null),
                requirements: requirements.clone(),
            },
            reason: CancelReason::HandlerFailed,
            error: None,
            response_status: None,
            settled_phases: vec![SettlePhase::BeforeHandler],
        };
        scheme.settle_on_cancel(&ctx).await.is_some()
    }

    /// Registers a protocol extension advertised on 402 responses.
    #[must_use]
    pub fn with_extension(mut self, extension: impl Extension + 'static) -> Self {
        self.extensions.register(extension);
        self
    }
}

/// Checks each accept against facilitator `/supported` kinds.
///
/// Unused registrations are ignored. An unregistered accept skips
/// [`SchemeNetworkServer::validate_facilitator_support`].
///
/// # Errors
///
/// [`FacilitatorSupportError::KindMissing`] when no v2 kind matches the
/// accept. Other variants come from the registered scheme hook.
pub fn validate_accepts_against_supported(
    server: &ResourceServer,
    accepts: &[PaymentRequirements],
    supported: &SupportedResponse,
) -> Result<(), FacilitatorSupportError> {
    for requirements in accepts {
        let scheme = requirements.scheme.as_str();
        let network = requirements.network.to_string();
        let kind = supported.kinds.iter().find(|kind| {
            V2 == kind.x402_version
                && kind.scheme.as_str() == scheme
                && kind.network.as_str() == network
        });
        let Some(kind) = kind else {
            return Err(FacilitatorSupportError::KindMissing {
                scheme: requirements.scheme.clone(),
                network: requirements.network.clone(),
            });
        };
        let Some(registered) = server.registered_scheme(scheme, &requirements.network) else {
            continue;
        };
        registered.validate_facilitator_support(
            &requirements.network,
            kind,
            &supported.extensions,
        )?;
    }
    Ok(())
}
