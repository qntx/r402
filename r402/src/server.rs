//! x402 resource server base logic.
//!
//! Contains the async-first x402 resource server, including scheme server
//! registration, facilitator client initialization, payment requirement
//! building, and verify/settle delegation with full hook lifecycle.
//!
//! Corresponds to Python SDK's `server_base.py` + `server.py`.

use std::collections::HashMap;

use crate::proto::helpers::find_schemes_by_network;
use crate::proto::{
    Network, PaymentPayload, PaymentRequired, PaymentRequirements, ResourceInfo, SettleResponse,
    SupportedKind, SupportedResponse, VerifyResponse,
};

use crate::config::ResourceConfig;
use crate::error::{PaymentAbortedError, SchemeNotFoundError};
use crate::hooks::{
    AbortResult, PayloadView, RecoveredSettleResult, RecoveredVerifyResult, RequirementsView,
    SettleContext, SettleFailureContext, SettleResultContext, VerifyContext, VerifyFailureContext,
    VerifyResultContext,
};
use crate::scheme::{AssetAmount, BoxFuture, SchemeError, SchemeServer};

/// Async facilitator client trait for resource servers.
///
/// Resource servers delegate verify/settle to a remote facilitator via this
/// trait. Implementations typically make HTTP calls to a facilitator service.
///
/// All methods are async (returning [`BoxFuture`]) because the primary
/// implementation (`HttpFacilitatorClient`) performs network I/O.
///
/// Corresponds to Python SDK's `FacilitatorClient` protocol in
/// `facilitator_client_base.py`.
pub trait FacilitatorClient: Send + Sync {
    /// Verifies a V2 payment asynchronously.
    fn verify<'a>(
        &'a self,
        payload: &'a PaymentPayload,
        requirements: &'a PaymentRequirements,
    ) -> BoxFuture<'a, Result<VerifyResponse, SchemeError>>;

    /// Settles a V2 payment asynchronously.
    fn settle<'a>(
        &'a self,
        payload: &'a PaymentPayload,
        requirements: &'a PaymentRequirements,
    ) -> BoxFuture<'a, Result<SettleResponse, SchemeError>>;

    /// Returns the supported payment kinds asynchronously.
    ///
    /// Called during [`X402ResourceServer::initialize`] to discover which
    /// (scheme, network) pairs the facilitator can handle.
    fn get_supported(&self) -> BoxFuture<'_, Result<SupportedResponse, SchemeError>>;
}

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

/// Extension interface for enriching payment declarations with
/// transport-specific data (e.g., HTTP request context).
///
/// Corresponds to Python SDK's `ResourceServerExtension` in
/// `schemas/extensions.py`.
pub trait ResourceServerExtension: Send + Sync {
    /// Unique extension key (e.g., `"bazaar"`).
    fn key(&self) -> &str;

    /// Enriches an extension declaration with transport-specific data.
    ///
    /// Called by the HTTP server middleware before building the 402 response.
    ///
    /// - `declaration` — the extension declaration from the route config.
    /// - `transport_context` — opaque transport context (e.g., serialized
    ///   HTTP request metadata).
    fn enrich_declaration(
        &self,
        declaration: serde_json::Value,
        transport_context: &serde_json::Value,
    ) -> serde_json::Value;
}

/// Async-first x402 resource server with scheme registration, facilitator
/// client initialization, requirement building, and verify/settle delegation.
///
/// Corresponds to Python SDK's `x402ResourceServer`.
pub struct X402ResourceServer {
    facilitator_clients: Vec<Box<dyn FacilitatorClient>>,
    schemes: HashMap<Network, HashMap<String, Box<dyn SchemeServer>>>,
    facilitator_map: HashMap<Network, HashMap<String, usize>>,
    supported_responses: HashMap<Network, HashMap<String, SupportedResponse>>,
    extensions: HashMap<String, Box<dyn ResourceServerExtension>>,
    before_verify_hooks: Vec<BeforeVerifyHook>,
    after_verify_hooks: Vec<AfterVerifyHook>,
    on_verify_failure_hooks: Vec<OnVerifyFailureHook>,
    before_settle_hooks: Vec<BeforeSettleHook>,
    after_settle_hooks: Vec<AfterSettleHook>,
    on_settle_failure_hooks: Vec<OnSettleFailureHook>,
    initialized: bool,
}

impl std::fmt::Debug for X402ResourceServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X402ResourceServer")
            .field("facilitator_clients_count", &self.facilitator_clients.len())
            .field("schemes_networks", &self.schemes.keys().collect::<Vec<_>>())
            .field("initialized", &self.initialized)
            .finish_non_exhaustive()
    }
}

impl Default for X402ResourceServer {
    fn default() -> Self {
        Self::new()
    }
}

impl X402ResourceServer {
    /// Creates a new resource server with no facilitator clients.
    #[must_use]
    pub fn new() -> Self {
        Self {
            facilitator_clients: Vec::new(),
            schemes: HashMap::new(),
            facilitator_map: HashMap::new(),
            supported_responses: HashMap::new(),
            extensions: HashMap::new(),
            before_verify_hooks: Vec::new(),
            after_verify_hooks: Vec::new(),
            on_verify_failure_hooks: Vec::new(),
            before_settle_hooks: Vec::new(),
            after_settle_hooks: Vec::new(),
            on_settle_failure_hooks: Vec::new(),
            initialized: false,
        }
    }

    /// Creates a new resource server with one facilitator client.
    #[must_use]
    pub fn with_facilitator(client: Box<dyn FacilitatorClient>) -> Self {
        let mut server = Self::new();
        server.facilitator_clients.push(client);
        server
    }

    /// Adds a facilitator client.
    pub fn add_facilitator(&mut self, client: Box<dyn FacilitatorClient>) -> &mut Self {
        self.facilitator_clients.push(client);
        self
    }

    /// Registers a V2 scheme server for a network.
    pub fn register(&mut self, network: Network, server: Box<dyn SchemeServer>) -> &mut Self {
        let scheme = server.scheme().to_owned();
        self.schemes
            .entry(network)
            .or_default()
            .insert(scheme, server);
        self
    }

    /// Checks if a scheme is registered for a network (with wildcard matching).
    #[must_use]
    pub fn has_registered_scheme(&self, network: &str, scheme: &str) -> bool {
        if let Some(schemes) = self.schemes.get(network)
            && schemes.contains_key(scheme)
        {
            return true;
        }
        let prefix = network.split(':').next().unwrap_or("");
        let wildcard = format!("{prefix}:*");
        self.schemes
            .get(&wildcard)
            .is_some_and(|s| s.contains_key(scheme))
    }

    /// Returns the `SupportedKind` for a given version/network/scheme, or `None`.
    #[must_use]
    pub fn get_supported_kind(
        &self,
        version: u32,
        network: &str,
        scheme: &str,
    ) -> Option<&SupportedKind> {
        // Exact network match
        if let Some(kind) = self.find_kind_in(network, scheme, version, network) {
            return Some(kind);
        }
        // Wildcard match
        let prefix = network.split(':').next().unwrap_or("");
        let wildcard = format!("{prefix}:*");
        if let Some(kind) = self.find_kind_in(&wildcard, scheme, version, network) {
            return Some(kind);
        }
        // Scan all stored responses for wildcard kind patterns
        for schemes in self.supported_responses.values() {
            if let Some(supported) = schemes.get(scheme) {
                for kind in &supported.kinds {
                    if kind.x402_version == version
                        && kind.scheme == scheme
                        && kind.network.ends_with(":*")
                    {
                        let kind_prefix = kind.network.split(':').next().unwrap_or("");
                        if network.starts_with(&format!("{kind_prefix}:")) {
                            return Some(kind);
                        }
                    }
                }
            }
        }
        None
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

    /// Registers a [`ResourceServerExtension`].
    ///
    /// Extensions enrich payment declarations with transport-specific data
    /// (e.g., bazaar metadata from an HTTP request).
    pub fn register_extension(&mut self, ext: Box<dyn ResourceServerExtension>) -> &mut Self {
        self.extensions.insert(ext.key().to_owned(), ext);
        self
    }

    /// Enriches extension declarations using registered extensions.
    ///
    /// For each key in `declarations` that has a matching registered
    /// extension, calls [`ResourceServerExtension::enrich_declaration`]
    /// with the given `transport_context`.
    ///
    /// Returns the enriched extensions object.
    #[must_use]
    pub fn enrich_extensions(
        &self,
        declarations: &serde_json::Value,
        transport_context: &serde_json::Value,
    ) -> serde_json::Value {
        let Some(obj) = declarations.as_object() else {
            return declarations.clone();
        };

        let mut result = obj.clone();
        for (key, value) in obj {
            if let Some(ext) = self.extensions.get(key) {
                result.insert(
                    key.clone(),
                    ext.enrich_declaration(value.clone(), transport_context),
                );
            }
        }

        serde_json::Value::Object(result)
    }

    /// Initializes the server by fetching supported kinds from all
    /// registered facilitator clients.
    ///
    /// Must be called before `verify_payment` / `settle_payment`.
    ///
    /// # Errors
    ///
    /// Returns an error if any facilitator client fails to respond.
    pub async fn initialize(&mut self) -> Result<(), SchemeError> {
        for (idx, client) in self.facilitator_clients.iter().enumerate() {
            let supported = client.get_supported().await?;

            for kind in &supported.kinds {
                let network = &kind.network;
                let scheme = &kind.scheme;

                self.facilitator_map
                    .entry(network.clone())
                    .or_default()
                    .entry(scheme.clone())
                    .or_insert(idx);

                self.supported_responses
                    .entry(network.clone())
                    .or_default()
                    .entry(scheme.clone())
                    .or_insert_with(|| supported.clone());
            }
        }

        self.initialized = true;
        Ok(())
    }

    /// Returns whether the server has been initialized.
    #[must_use]
    pub const fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Builds payment requirements for a protected resource.
    ///
    /// # Errors
    ///
    /// Returns an error if the server is not initialized, the scheme is not
    /// registered, or price parsing fails.
    pub fn build_payment_requirements(
        &self,
        config: &ResourceConfig,
    ) -> Result<Vec<PaymentRequirements>, SchemeError> {
        if !self.initialized {
            return Err("Server not initialized. Call initialize() first.".into());
        }

        let schemes = find_schemes_by_network(&self.schemes, &config.network)
            .ok_or_else(|| SchemeNotFoundError::new(&config.scheme, &config.network))?;

        let server = schemes
            .get(&config.scheme)
            .ok_or_else(|| SchemeNotFoundError::new(&config.scheme, &config.network))?;

        let supported = self
            .supported_responses
            .get(&config.network)
            .and_then(|m| m.get(&config.scheme))
            .ok_or_else(|| SchemeNotFoundError::new(&config.scheme, &config.network))?;

        let supported_kind = supported
            .kinds
            .iter()
            .find(|k| k.scheme == config.scheme && k.network == config.network)
            .ok_or_else(|| SchemeNotFoundError::new(&config.scheme, &config.network))?;

        let AssetAmount {
            amount,
            asset,
            extra,
        } = server.parse_price(&config.price, &config.network)?;

        let base = PaymentRequirements {
            scheme: config.scheme.clone(),
            network: config.network.clone(),
            asset,
            amount,
            pay_to: config.pay_to.clone(),
            max_timeout_seconds: config.max_timeout_seconds.unwrap_or(300),
            extra: extra.unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new())),
        };

        let enhanced = server.enhance_payment_requirements(base, supported_kind, &[]);

        Ok(vec![enhanced])
    }

    /// Creates a 402 Payment Required response from a list of requirements.
    #[must_use]
    pub const fn create_payment_required(
        &self,
        requirements: Vec<PaymentRequirements>,
        resource: Option<ResourceInfo>,
        error: Option<String>,
        extensions: Option<serde_json::Value>,
    ) -> PaymentRequired {
        PaymentRequired {
            x402_version: 2,
            error,
            resource,
            accepts: requirements,
            extensions,
        }
    }

    /// Finds requirements from a list that match a given payload.
    #[must_use]
    pub fn find_matching_requirements<'a>(
        &self,
        available: &'a [PaymentRequirements],
        payload: &PaymentPayload,
    ) -> Option<&'a PaymentRequirements> {
        available.iter().find(|req| {
            payload.accepted.scheme == req.scheme
                && payload.accepted.network == req.network
                && payload.accepted.amount == req.amount
                && payload.accepted.asset == req.asset
                && payload.accepted.pay_to == req.pay_to
        })
    }

    /// Verifies a V2 payment via the appropriate facilitator client,
    /// with full hook lifecycle.
    ///
    /// # Errors
    ///
    /// Returns an error if the server is not initialized, no facilitator is
    /// registered, or a hook aborts.
    pub async fn verify_payment(
        &self,
        payload: &PaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<VerifyResponse, SchemeError> {
        if !self.initialized {
            return Err("Server not initialized. Call initialize() first.".into());
        }

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

        let result = self.do_verify(payload, requirements).await;

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

    /// Settles a V2 payment via the appropriate facilitator client,
    /// with full hook lifecycle.
    ///
    /// # Errors
    ///
    /// Returns an error if the server is not initialized, no facilitator is
    /// registered, or a hook aborts.
    pub async fn settle_payment(
        &self,
        payload: &PaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<SettleResponse, SchemeError> {
        if !self.initialized {
            return Err("Server not initialized. Call initialize() first.".into());
        }

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

        let result = self.do_settle(payload, requirements).await;

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

    /// Looks up a `SupportedKind` from stored responses.
    fn find_kind_in<'a>(
        &'a self,
        stored_network: &str,
        scheme: &str,
        version: u32,
        target_network: &str,
    ) -> Option<&'a SupportedKind> {
        self.supported_responses
            .get(stored_network)
            .and_then(|m| m.get(scheme))
            .and_then(|supported| {
                supported.kinds.iter().find(|k| {
                    k.x402_version == version
                        && k.scheme == scheme
                        && (k.network == target_network || k.network == stored_network)
                })
            })
    }

    /// Delegates verify to the facilitator client for the given scheme/network.
    async fn do_verify(
        &self,
        payload: &PaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<VerifyResponse, SchemeError> {
        let client = self.find_facilitator_client(payload.scheme(), payload.network())?;
        client.verify(payload, requirements).await
    }

    /// Delegates settle to the facilitator client for the given scheme/network.
    async fn do_settle(
        &self,
        payload: &PaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<SettleResponse, SchemeError> {
        let client = self.find_facilitator_client(payload.scheme(), payload.network())?;
        client.settle(payload, requirements).await
    }

    /// Finds the facilitator client index for a given scheme/network.
    fn find_facilitator_client(
        &self,
        scheme: &str,
        network: &str,
    ) -> Result<&dyn FacilitatorClient, SchemeError> {
        let idx = self
            .facilitator_map
            .get(network)
            .and_then(|m| m.get(scheme))
            .copied()
            .ok_or_else(|| SchemeNotFoundError::new(scheme, network))?;

        self.facilitator_clients
            .get(idx)
            .map(AsRef::as_ref)
            .ok_or_else(|| {
                let err: SchemeError = Box::new(SchemeNotFoundError::new(scheme, network));
                err
            })
    }
}
