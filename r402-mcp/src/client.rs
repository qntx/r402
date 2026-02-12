//! Client-side MCP x402 payment handling.
//!
//! This module provides [`X402McpClient`] which wraps an MCP caller with
//! automatic x402 payment handling. When a tool returns a 402 payment
//! required response, the client automatically creates a payment, attaches
//! it to the request, and retries.
//!
//! # Architecture
//!
//! The client uses the [`McpCaller`] trait to abstract over MCP SDK implementations.
//! Payment creation is delegated to [`SchemeClient`](r402::scheme::SchemeClient)
//! instances, with candidate selection controlled by [`PaymentSelector`](r402::scheme::PaymentSelector)
//! and [`PaymentPolicy`](r402::scheme::PaymentPolicy).

use r402::facilitator::BoxFuture;
use r402::proto;
use r402::scheme::{
    ClientError, FirstMatch, PaymentCandidate, PaymentPolicy, PaymentSelector, SchemeClient,
};

use crate::PAYMENT_META_KEY;
use crate::error::{McpPaymentError, PaymentRequiredError};
use crate::extract;
use crate::types::{
    AfterPaymentContext, CallToolParams, CallToolResult, ClientHooks, ClientOptions, NoClientHooks,
    PaidToolCallResult, PaymentRequiredContext,
};

/// Trait abstracting MCP tool call capability.
///
/// Implement this trait to integrate with any MCP SDK. The implementation
/// should forward `call_tool` to the underlying MCP session/client.
///
/// # Examples
///
/// ```rust,ignore
/// struct MyMcpSession { /* ... */ }
///
/// impl McpCaller for MyMcpSession {
///     fn call_tool(
///         &self,
///         params: CallToolParams,
///     ) -> BoxFuture<'_, Result<CallToolResult, McpPaymentError>> {
///         Box::pin(async move {
///             // Forward to actual MCP SDK
///             todo!()
///         })
///     }
/// }
/// ```
pub trait McpCaller: Send + Sync {
    /// Calls an MCP tool with the given parameters.
    fn call_tool(
        &self,
        params: CallToolParams,
    ) -> BoxFuture<'_, Result<CallToolResult, McpPaymentError>>;
}

/// An x402-aware MCP client with automatic payment handling.
///
/// Wraps an [`McpCaller`] with payment scheme clients and selection logic.
/// When a tool returns a 402 payment required response, the client:
///
/// 1. Extracts payment requirements from the error result
/// 2. Generates payment candidates via registered [`SchemeClient`]s
/// 3. Applies policies and selects the best candidate
/// 4. Signs the payment and retries with payment in `_meta`
/// 5. Extracts settlement response from the result
pub struct X402McpClient<C: McpCaller> {
    caller: C,
    scheme_clients: Vec<Box<dyn SchemeClient>>,
    selector: Box<dyn PaymentSelector>,
    policies: Vec<Box<dyn PaymentPolicy>>,
    options: ClientOptions,
    hooks: Box<dyn ClientHooks>,
}

impl<C: McpCaller> std::fmt::Debug for X402McpClient<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X402McpClient")
            .field("scheme_clients", &self.scheme_clients.len())
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl<C: McpCaller> X402McpClient<C> {
    /// Creates a builder for configuring an [`X402McpClient`].
    pub fn builder(caller: C) -> X402McpClientBuilder<C> {
        X402McpClientBuilder {
            caller,
            scheme_clients: Vec::new(),
            selector: None,
            policies: Vec::new(),
            options: ClientOptions::default(),
            hooks: None,
        }
    }

    /// Returns a reference to the underlying MCP caller.
    pub const fn caller(&self) -> &C {
        &self.caller
    }

    /// Calls a tool with automatic x402 payment handling.
    ///
    /// # Flow
    ///
    /// 1. Calls the tool without payment
    /// 2. If the server returns a payment required error, creates a payment
    /// 3. Retries with payment attached in `_meta`
    /// 4. Returns the result with payment response extracted
    ///
    /// # Errors
    ///
    /// Returns [`McpPaymentError`] if the tool call fails, payment creation
    /// fails, or a hook aborts the operation.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Map<String, serde_json::Value>,
    ) -> Result<PaidToolCallResult, McpPaymentError> {
        let params = CallToolParams {
            name: name.to_owned(),
            arguments: arguments.clone(),
            meta: None,
        };

        let result = self.caller.call_tool(params).await?;

        // If not an error, return directly
        if !result.is_error {
            return Ok(build_paid_result(result, false));
        }

        // Try to extract payment required from the error
        let payment_required = match extract::extract_payment_required_from_result(&result) {
            Some(pr) if !pr.accepts.is_empty() => pr,
            _ => return Ok(build_paid_result(result, false)),
        };

        let pr_ctx = PaymentRequiredContext {
            tool_name: name.to_owned(),
            arguments: arguments.clone(),
            payment_required: payment_required.clone(),
        };

        // on_payment_required hook — can provide custom payment or abort
        let custom_payment = self.hooks.on_payment_required(&pr_ctx).await?;
        if let Some(payload) = custom_payment {
            return self.call_tool_with_payload(name, arguments, payload).await;
        }

        // Check auto-payment
        if !self.options.auto_payment {
            return Err(McpPaymentError::PaymentRequired(Box::new(
                PaymentRequiredError::new("Payment required", payment_required),
            )));
        }

        // on_payment_requested hook — can approve/deny
        let approved = self.hooks.on_payment_requested(&pr_ctx).await?;
        if !approved {
            return Err(McpPaymentError::PaymentRequired(Box::new(
                PaymentRequiredError::new("Payment denied by hook", payment_required),
            )));
        }

        // Create payment using scheme clients
        let payload = self.create_payment(&payment_required).await?;

        self.call_tool_with_payload(name, arguments, payload).await
    }

    /// Calls a tool with a pre-created payment payload.
    ///
    /// # Errors
    ///
    /// Returns [`McpPaymentError`] if the tool call fails.
    pub async fn call_tool_with_payment(
        &self,
        name: &str,
        arguments: serde_json::Map<String, serde_json::Value>,
        payload: serde_json::Value,
    ) -> Result<PaidToolCallResult, McpPaymentError> {
        self.call_tool_with_payload(name, arguments, payload).await
    }

    /// Fetches payment requirements for a tool without paying.
    ///
    /// Calls the tool and extracts the [`proto::PaymentRequired`] from the
    /// error response, if any.
    ///
    /// # Errors
    ///
    /// Returns [`McpPaymentError`] if the tool call fails.
    pub async fn get_tool_payment_requirements(
        &self,
        name: &str,
        arguments: serde_json::Map<String, serde_json::Value>,
    ) -> Result<Option<proto::PaymentRequired>, McpPaymentError> {
        let params = CallToolParams {
            name: name.to_owned(),
            arguments,
            meta: None,
        };

        let result = self.caller.call_tool(params).await?;
        Ok(extract::extract_payment_required_from_result(&result))
    }

    async fn call_tool_with_payload(
        &self,
        name: &str,
        arguments: serde_json::Map<String, serde_json::Value>,
        payload: serde_json::Value,
    ) -> Result<PaidToolCallResult, McpPaymentError> {
        let mut meta = serde_json::Map::new();
        meta.insert(PAYMENT_META_KEY.to_owned(), payload.clone());

        let params = CallToolParams {
            name: name.to_owned(),
            arguments,
            meta: Some(meta),
        };

        let result = self.caller.call_tool(params).await?;

        // on_after_payment hook
        let settle_response = result
            .meta
            .as_ref()
            .and_then(extract::extract_payment_response_from_meta);

        let after_ctx = AfterPaymentContext {
            tool_name: name.to_owned(),
            payment_payload: payload,
            result: result.clone(),
            settle_response: settle_response.clone(),
        };
        // Non-fatal: ignore hook errors
        let _ = self.hooks.on_after_payment(&after_ctx).await;

        Ok(build_paid_result(result, true))
    }

    async fn create_payment(
        &self,
        payment_required: &proto::PaymentRequired,
    ) -> Result<serde_json::Value, McpPaymentError> {
        // Collect candidates from all scheme clients
        let mut candidates: Vec<PaymentCandidate> = Vec::new();
        for client in &self.scheme_clients {
            candidates.extend(client.accept(payment_required));
        }

        if candidates.is_empty() {
            return Err(McpPaymentError::NoMatchingPaymentOption);
        }

        // Apply policies
        let mut refs: Vec<&PaymentCandidate> = candidates.iter().collect();
        for policy in &self.policies {
            refs = policy.apply(refs);
        }

        // Select best candidate
        let selected = self
            .selector
            .select(&refs)
            .ok_or(McpPaymentError::NoMatchingPaymentOption)?;

        // Sign the payment
        let signed_json = selected.sign().await.map_err(|e| match e {
            ClientError::SigningError(msg) => McpPaymentError::SigningFailed(msg),
            other => McpPaymentError::PaymentCreationFailed(other.to_string()),
        })?;

        // Parse the signed JSON string into a Value for meta embedding
        let payload: serde_json::Value = serde_json::from_str(&signed_json)?;
        Ok(payload)
    }
}

/// Builder for configuring an [`X402McpClient`].
pub struct X402McpClientBuilder<C: McpCaller> {
    caller: C,
    scheme_clients: Vec<Box<dyn SchemeClient>>,
    selector: Option<Box<dyn PaymentSelector>>,
    policies: Vec<Box<dyn PaymentPolicy>>,
    options: ClientOptions,
    hooks: Option<Box<dyn ClientHooks>>,
}

impl<C: McpCaller> std::fmt::Debug for X402McpClientBuilder<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X402McpClientBuilder")
            .field("scheme_clients", &self.scheme_clients.len())
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl<C: McpCaller> X402McpClientBuilder<C> {
    /// Registers a payment scheme client.
    #[must_use]
    pub fn scheme_client(mut self, client: Box<dyn SchemeClient>) -> Self {
        self.scheme_clients.push(client);
        self
    }

    /// Sets the payment selector strategy.
    ///
    /// Defaults to [`FirstMatch`] if not set.
    #[must_use]
    pub fn selector(mut self, selector: Box<dyn PaymentSelector>) -> Self {
        self.selector = Some(selector);
        self
    }

    /// Adds a payment policy to the filtering pipeline.
    #[must_use]
    pub fn policy(mut self, policy: Box<dyn PaymentPolicy>) -> Self {
        self.policies.push(policy);
        self
    }

    /// Sets client options.
    #[must_use]
    pub const fn options(mut self, options: ClientOptions) -> Self {
        self.options = options;
        self
    }

    /// Enables or disables automatic payment handling.
    #[must_use]
    pub const fn auto_payment(mut self, enabled: bool) -> Self {
        self.options.auto_payment = enabled;
        self
    }

    /// Sets lifecycle hooks for payment events.
    #[must_use]
    pub fn hooks(mut self, hooks: Box<dyn ClientHooks>) -> Self {
        self.hooks = Some(hooks);
        self
    }

    /// Builds the configured [`X402McpClient`].
    ///
    /// # Panics
    ///
    /// Panics if no scheme clients have been registered.
    #[must_use]
    pub fn build(self) -> X402McpClient<C> {
        assert!(
            !self.scheme_clients.is_empty(),
            "at least one scheme client must be registered"
        );
        X402McpClient {
            caller: self.caller,
            scheme_clients: self.scheme_clients,
            selector: self.selector.unwrap_or_else(|| Box::new(FirstMatch)),
            policies: self.policies,
            options: self.options,
            hooks: self.hooks.unwrap_or_else(|| Box::new(NoClientHooks)),
        }
    }
}

/// Standalone function to make a paid MCP tool call.
///
/// This is a convenience function for simple use cases where you don't need
/// the full [`X402McpClient`] builder. It calls the tool, detects 402 responses,
/// creates a payment from the first accepted requirement, and retries.
///
/// # Errors
///
/// Returns [`McpPaymentError`] if any step of the payment flow fails.
pub async fn call_paid_tool(
    caller: &dyn McpCaller,
    scheme_clients: &[&dyn SchemeClient],
    name: &str,
    arguments: serde_json::Map<String, serde_json::Value>,
) -> Result<PaidToolCallResult, McpPaymentError> {
    // First call without payment
    let params = CallToolParams {
        name: name.to_owned(),
        arguments: arguments.clone(),
        meta: None,
    };

    let result = caller.call_tool(params).await?;

    if !result.is_error {
        return Ok(build_paid_result(result, false));
    }

    let payment_required = match extract::extract_payment_required_from_result(&result) {
        Some(pr) if !pr.accepts.is_empty() => pr,
        _ => return Ok(build_paid_result(result, false)),
    };

    // Collect candidates from all scheme clients
    let mut candidates: Vec<PaymentCandidate> = Vec::new();
    for client in scheme_clients {
        candidates.extend(client.accept(&payment_required));
    }

    let refs: Vec<&PaymentCandidate> = candidates.iter().collect();
    let selected = FirstMatch
        .select(&refs)
        .ok_or(McpPaymentError::NoMatchingPaymentOption)?;

    let signed_json = selected.sign().await.map_err(|e| match e {
        ClientError::SigningError(msg) => McpPaymentError::SigningFailed(msg),
        other => McpPaymentError::PaymentCreationFailed(other.to_string()),
    })?;

    let payload: serde_json::Value = serde_json::from_str(&signed_json)?;

    // Retry with payment in _meta
    let mut meta = serde_json::Map::new();
    meta.insert(PAYMENT_META_KEY.to_owned(), payload);

    let params = CallToolParams {
        name: name.to_owned(),
        arguments,
        meta: Some(meta),
    };

    let result = caller.call_tool(params).await?;
    Ok(build_paid_result(result, true))
}

/// Converts a [`CallToolResult`] into a [`PaidToolCallResult`].
fn build_paid_result(result: CallToolResult, payment_made: bool) -> PaidToolCallResult {
    let payment_response = result
        .meta
        .as_ref()
        .and_then(extract::extract_payment_response_from_meta);

    PaidToolCallResult {
        content: result.content.clone(),
        is_error: result.is_error,
        payment_response,
        payment_made,
        raw_result: result,
    }
}
