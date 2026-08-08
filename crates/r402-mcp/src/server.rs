//! Server-side MCP payment wrapper.
//!
//! Wraps an async tool handler so unpaid calls receive a structured
//! payment-required error and paid calls run verify → handler → settle.

use std::future::Future;
use std::sync::Arc;

use r402_core::facilitator::Facilitator;
use r402_core::wire::{
    PaymentRequired, ResourceInfo, SettleRequest, SettleResponse, VerifyRequest,
};
use serde_json::{Value, json};

use crate::{MCP_PAYMENT_META_KEY, MCP_PAYMENT_REQUIRED_CODE, MCP_PAYMENT_RESPONSE_META_KEY};

/// Configuration for [`PaymentWrapper`].
#[derive(Debug, Clone)]
pub struct PaymentWrapperConfig {
    /// Accepts array used to build `PaymentRequired` when payment is missing.
    pub accepts: Vec<Value>,
    /// Resource info embedded in the payment-required payload.
    pub resource: ResourceInfo,
    /// When true, settle after the handler succeeds (sequential semantics).
    pub settle_on_success: bool,
}

impl PaymentWrapperConfig {
    /// Builds a config with the given accepts list and resource.
    #[must_use]
    pub const fn new(accepts: Vec<Value>, resource: ResourceInfo) -> Self {
        Self {
            accepts,
            resource,
            settle_on_success: true,
        }
    }

    /// Disables automatic settlement (verify-only gate).
    #[must_use]
    pub const fn without_settle(mut self) -> Self {
        self.settle_on_success = false;
        self
    }
}

/// Result of a payment-wrapped tool invocation.
#[derive(Debug, Clone)]
pub struct WrappedToolResult {
    /// Whether the tool reported an error (including payment-required).
    pub is_error: bool,
    /// Free-form tool content (opaque to r402).
    pub content: Value,
    /// MCP `_meta` map (may contain payment / payment-response).
    pub meta: Value,
}

/// Wraps tool handlers with x402 verify / settle.
#[derive(Debug)]
pub struct PaymentWrapper<F> {
    facilitator: Arc<F>,
    config: PaymentWrapperConfig,
}

impl<F> PaymentWrapper<F>
where
    F: Facilitator + Send + Sync,
{
    /// Creates a new wrapper around `facilitator`.
    #[must_use]
    pub const fn new(facilitator: Arc<F>, config: PaymentWrapperConfig) -> Self {
        Self {
            facilitator,
            config,
        }
    }

    /// Builds a payment-required tool result (no handler invocation).
    #[must_use]
    pub fn payment_required_result(&self, error_detail: impl Into<String>) -> WrappedToolResult {
        let accepts: Vec<_> = self
            .config
            .accepts
            .iter()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .collect();
        let required = PaymentRequired::new(self.config.resource.clone())
            .with_error(error_detail.into())
            .with_accepts(accepts);
        let payment_json = serde_json::to_value(&required).unwrap_or_else(|_| json!({}));
        WrappedToolResult {
            is_error: true,
            content: json!([{
                "type": "text",
                "text": MCP_PAYMENT_REQUIRED_CODE,
            }]),
            meta: json!({
                MCP_PAYMENT_META_KEY: payment_json,
            }),
        }
    }

    /// Runs verify (+ optional settle) around `handler`.
    ///
    /// `payment_payload` is the client payment JSON from tool request
    /// `_meta["x402/payment"]`. `requirements` defaults to the first accept.
    pub async fn invoke<H, Fut>(
        &self,
        payment_payload: Option<Value>,
        requirements: Option<Value>,
        handler: H,
    ) -> WrappedToolResult
    where
        H: FnOnce() -> Fut,
        Fut: Future<Output = WrappedToolResult>,
    {
        let Some(payload) = payment_payload else {
            return self.payment_required_result("missing payment in tool _meta");
        };
        let Some(requirements) = requirements.or_else(|| self.config.accepts.first().cloned())
        else {
            return self.payment_required_result("no payment requirements configured");
        };

        let verify_body = json!({
            "x402Version": 2,
            "paymentPayload": payload,
            "paymentRequirements": requirements,
        });
        let verify_req = VerifyRequest::from(verify_body.clone());

        if let Err(err) = self.facilitator.verify(verify_req).await {
            return self.payment_required_result(err.to_string());
        }

        let mut result = handler().await;
        if result.is_error {
            return result;
        }

        if self.config.settle_on_success {
            result = self.attach_settlement(result, verify_body).await;
        }

        result
    }

    async fn attach_settlement(
        &self,
        mut result: WrappedToolResult,
        verify_body: Value,
    ) -> WrappedToolResult {
        let settle_req = SettleRequest::from(verify_body);
        match self.facilitator.settle(settle_req).await {
            Ok(settle) if settle.is_success() => {
                Self::insert_payment_response_meta(&mut result, &settle);
                result
            }
            Ok(_) | Err(_) => self.payment_required_result("settlement failed"),
        }
    }

    fn insert_payment_response_meta(result: &mut WrappedToolResult, settle: &SettleResponse) {
        let Ok(v) = serde_json::to_value(settle) else {
            return;
        };
        if let Some(obj) = result.meta.as_object_mut() {
            obj.insert(MCP_PAYMENT_RESPONSE_META_KEY.to_owned(), v);
        } else {
            result.meta = json!({ MCP_PAYMENT_RESPONSE_META_KEY: v });
        }
    }
}
