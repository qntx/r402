//! Server-side MCP payment wrapper (Go `PaymentWrapper`).
//!
//! MCP v1 does not expose [`r402_server::SettlementMode`]; only `paymentFlow`.

mod flow;

use std::future::Future;
use std::sync::Arc;

use r402_facilitator::DynFacilitator;
use r402_protocol::error::FacilitatorError;
use r402_protocol::payment::{
    Extensions, PaymentRequired, PaymentRequirements, ResourceInfo, SettleResponse,
};
use r402_server::{
    PaymentFlowError, PaymentFlowName, PaymentRequiredBuildContext, ResourceServer,
    SkipHandlerDirective,
};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock};
use serde_json::Value;

use crate::MCP_TOOL_URL_PREFIX;
use crate::tool::{
    McpPaymentPayload, attach_settle_response, create_tool_resource_url,
    payment_required_tool_result, settlement_failed_tool_result,
};

/// Error constructing a [`PaymentWrapper`].
#[derive(Debug, Clone, thiserror::Error)]
pub enum PaymentWrapperConfigError {
    /// `accepts` must be non-empty (Go panics; we return an error).
    #[error("PaymentWrapperConfig.accepts must have at least one payment requirement")]
    EmptyAccepts,
    /// No [`r402_server::SchemeNetworkServer`] registered for this accept.
    #[error("missing scheme {scheme} on {network}")]
    MissingScheme {
        /// Wire scheme name.
        scheme: String,
        /// Accept CAIP-2 network.
        network: String,
    },
    /// [`r402_server::resolve_payment_flow`] failed for an accept.
    #[error(transparent)]
    UnresolvedPaymentFlow(#[from] PaymentFlowError),
    /// Escrow accept whose scheme does not implement `settle_on_cancel`.
    #[error("escrow scheme {scheme} is missing settle_on_cancel")]
    MissingSettleOnCancel {
        /// Wire scheme name.
        scheme: String,
    },
}

fn settle_failure_reason(settle: &SettleResponse) -> String {
    match settle {
        SettleResponse::Failure {
            reason, message, ..
        } => message
            .as_ref()
            .map_or_else(|| reason.to_string(), ToString::to_string),
        _ => "Settlement failed".into(),
    }
}

fn internal_server_error_result(settle: Option<&SettleResponse>) -> CallToolResult {
    let result = CallToolResult::error(vec![ContentBlock::text("Internal Server Error")]);
    match settle {
        Some(settle) => attach_settle_response(result, settle),
        None => result,
    }
}

fn skip_handler_tool_result(directive: &SkipHandlerDirective) -> CallToolResult {
    match &directive.body {
        Some(Value::String(text)) => {
            CallToolResult::success(vec![ContentBlock::text(text.clone())])
        }
        Some(value) => CallToolResult::structured(value.clone()),
        None => CallToolResult::structured(Value::Null),
    }
}

fn empty_success_settle(requirements: &PaymentRequirements) -> SettleResponse {
    SettleResponse::Success {
        payer: String::new().into(),
        transaction: String::new().into(),
        network: requirements.network.to_string().into(),
        amount: None,
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    }
}

/// Server-side lifecycle hooks (Go `PaymentWrapperHooks`).
#[derive(Clone, Default)]
pub struct PaymentWrapperHooks {
    /// After verify, before tool. Return `false` to abort.
    pub on_before_execution: Option<Arc<dyn Fn(ServerHookContext) -> bool + Send + Sync>>,
    /// After tool success, before settle (non-fatal).
    pub on_after_execution: Option<Arc<dyn Fn(AfterExecutionContext) + Send + Sync>>,
    /// After successful settle (non-fatal).
    pub on_after_settlement: Option<Arc<dyn Fn(SettlementContext) + Send + Sync>>,
}

impl std::fmt::Debug for PaymentWrapperHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaymentWrapperHooks")
            .field("on_before_execution", &self.on_before_execution.is_some())
            .field("on_after_execution", &self.on_after_execution.is_some())
            .field("on_after_settlement", &self.on_after_settlement.is_some())
            .finish()
    }
}

/// Go `ServerHookContext`.
#[derive(Debug, Clone)]
pub struct ServerHookContext {
    /// Tool name.
    pub tool_name: String,
    /// Tool arguments (JSON object keys).
    pub arguments: serde_json::Map<String, Value>,
    /// Matched requirements.
    pub payment_requirements: PaymentRequirements,
    /// Client payment payload.
    pub payment_payload: McpPaymentPayload,
}

/// Go `AfterExecutionContext`.
#[derive(Debug, Clone)]
pub struct AfterExecutionContext {
    /// Shared server context.
    pub server: ServerHookContext,
    /// Tool result before settle.
    pub result: CallToolResult,
}

/// Go `SettlementContext`.
#[derive(Debug, Clone)]
pub struct SettlementContext {
    /// Shared server context.
    pub server: ServerHookContext,
    /// Successful settlement response.
    pub settlement: SettleResponse,
}

/// Go `PaymentWrapperConfig`.
#[derive(Debug, Clone)]
pub struct PaymentWrapperConfig {
    /// Advertised payment options.
    pub accepts: Vec<PaymentRequirements>,
    /// Resource metadata (defaulted if None at use sites).
    pub resource: Option<ResourceInfo>,
    /// Optional lifecycle hooks.
    pub hooks: PaymentWrapperHooks,
    /// Extensions included on payment-required responses.
    pub extensions: Extensions,
}

impl PaymentWrapperConfig {
    /// Builds config; returns error if `accepts` is empty.
    ///
    /// # Errors
    ///
    /// [`PaymentWrapperConfigError::EmptyAccepts`] when `accepts` is empty.
    pub fn try_new(
        accepts: Vec<PaymentRequirements>,
        resource: Option<ResourceInfo>,
    ) -> Result<Self, PaymentWrapperConfigError> {
        if accepts.is_empty() {
            return Err(PaymentWrapperConfigError::EmptyAccepts);
        }
        Ok(Self {
            accepts,
            resource,
            hooks: PaymentWrapperHooks::default(),
            extensions: Extensions::new(),
        })
    }

    /// Attaches hooks.
    #[must_use]
    pub fn with_hooks(mut self, hooks: PaymentWrapperHooks) -> Self {
        self.hooks = hooks;
        self
    }

    /// Attaches extensions for payment-required responses.
    #[must_use]
    pub fn with_extensions(mut self, extensions: Extensions) -> Self {
        self.extensions = extensions;
        self
    }
}

/// Go `PaymentWrapper`.
#[derive(Debug, Clone)]
pub struct PaymentWrapper {
    server: ResourceServer,
    config: PaymentWrapperConfig,
}

impl PaymentWrapper {
    /// Go `NewPaymentWrapper`.
    ///
    /// Requires a registered scheme and a resolvable payment flow per accept.
    /// Escrow additionally requires `settle_on_cancel`.
    ///
    /// # Errors
    ///
    /// Empty accepts, missing scheme, unresolved flow, or escrow without
    /// `settle_on_cancel`.
    pub async fn try_new(
        server: ResourceServer,
        config: PaymentWrapperConfig,
    ) -> Result<Self, PaymentWrapperConfigError> {
        if config.accepts.is_empty() {
            return Err(PaymentWrapperConfigError::EmptyAccepts);
        }
        for requirement in &config.accepts {
            if server
                .registered_scheme(requirement.scheme.as_str(), &requirement.network)
                .is_none()
            {
                return Err(PaymentWrapperConfigError::MissingScheme {
                    scheme: requirement.scheme.to_string(),
                    network: requirement.network.to_string(),
                });
            }
            let flow = server.get_payment_flow(requirement)?;
            if flow == PaymentFlowName::Escrow && !server.has_settle_on_cancel(requirement).await {
                return Err(PaymentWrapperConfigError::MissingSettleOnCancel {
                    scheme: requirement.scheme.to_string(),
                });
            }
        }
        Ok(Self { server, config })
    }

    fn build_tool_resource_info(&self, tool_name: &str) -> ResourceInfo {
        if let Some(info) = &self.config.resource {
            let mut info = info.clone();
            if info.url.is_empty() && !tool_name.is_empty() {
                info.url = create_tool_resource_url(tool_name, None).into();
            }
            if info.description.as_deref().is_none_or(str::is_empty) && !tool_name.is_empty() {
                info.description = Some(format!("Tool: {tool_name}").into());
            }
            if info.mime_type.as_deref().is_none_or(str::is_empty) {
                info.mime_type = Some("application/json".into());
            }
            return info;
        }
        if tool_name.is_empty() {
            ResourceInfo::new(format!("{MCP_TOOL_URL_PREFIX}unknown"))
                .with_description("Unknown tool")
                .with_mime_type("application/json")
        } else {
            ResourceInfo::new(create_tool_resource_url(tool_name, None))
                .with_description(format!("Tool: {tool_name}"))
                .with_mime_type("application/json")
        }
    }

    /// Go `paymentRequiredResult`.
    pub async fn payment_required_result(&self, error_msg: impl Into<String>) -> CallToolResult {
        self.payment_required_result_for("", error_msg, None).await
    }

    async fn payment_required_result_for(
        &self,
        tool_name: &str,
        error_msg: impl Into<String>,
        payload: Option<&McpPaymentPayload>,
    ) -> CallToolResult {
        match self
            .create_payment_required(tool_name, error_msg.into(), payload)
            .await
        {
            Ok(required) => payment_required_tool_result(&required),
            Err(err) => CallToolResult::structured_error(serde_json::json!({
                "error": err.to_string(),
            })),
        }
    }

    /// Go `settlementFailedResult` (same dual format as payment-required).
    pub async fn settlement_failed_result(&self, error_msg: impl Into<String>) -> CallToolResult {
        self.settlement_failed_result_for("", error_msg).await
    }

    async fn settlement_failed_result_for(
        &self,
        tool_name: &str,
        error_msg: impl Into<String>,
    ) -> CallToolResult {
        match self
            .create_payment_required(tool_name, error_msg.into(), None)
            .await
        {
            Ok(required) => settlement_failed_tool_result(&required),
            Err(err) => CallToolResult::structured_error(serde_json::json!({
                "error": err.to_string(),
            })),
        }
    }

    async fn create_payment_required(
        &self,
        tool_name: &str,
        error_msg: String,
        payload: Option<&McpPaymentPayload>,
    ) -> Result<PaymentRequired, FacilitatorError> {
        let supported = DynFacilitator::supported(self.server.facilitator().as_ref())
            .await
            .unwrap_or_default();
        self.server
            .create_payment_required_response(
                self.config.accepts.clone(),
                PaymentRequiredBuildContext {
                    resource: self.build_tool_resource_info(tool_name),
                    error: Some(error_msg.into()),
                    extensions: self.config.extensions.clone(),
                    supported,
                    payment_payload: payload.cloned(),
                },
            )
            .await
    }

    /// Go `Wrap`: cloneable async handler for rmcp tool routers.
    pub fn wrap<H, Fut>(
        &self,
        handler: H,
    ) -> impl Fn(
        CallToolRequestParams,
    ) -> std::pin::Pin<Box<dyn Future<Output = CallToolResult> + Send>>
    + Clone
    + Send
    + Sync
    + 'static
    where
        H: Fn(CallToolRequestParams) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = CallToolResult> + Send + 'static,
    {
        let this = self.clone();
        move |params| {
            let this = this.clone();
            let handler = handler.clone();
            Box::pin(async move { this.invoke(params, handler).await })
        }
    }
}
