//! paymentFlow pipeline for [`super::PaymentWrapper`].
//!
//! Facilitator [`r402_protocol::FacilitatorError::Transport`] is tool `isError`
//! without a `PaymentRequired` body (HTTP 502 analog). Buyer must not auto-pay it.

use std::future::Future;

use r402_protocol::error::{FacilitatorError, VerificationError};
use r402_protocol::payment::{PaymentRequirements, SettleResponse, VerifyResponse};
use r402_server::{
    CancelReason, CompletedSettlement, PaymentFlowName, SettlePhase, VerifyPaymentOutcome,
    build_failure_path_settlement_response, resolve_payment_flow_phases,
};
use rmcp::model::{CallToolRequestParams, CallToolResult};

use super::{
    AfterExecutionContext, PaymentWrapper, ServerHookContext, SettlementContext,
    empty_success_settle, facilitator_build_error, internal_server_error_result,
    settle_failure_reason, skip_handler_tool_result,
};
use crate::tool::{
    McpPaymentPayload, attach_settle_response, extract_payment_from_params,
    payment_required_tool_result,
};

impl PaymentWrapper {
    /// Full pipeline (Go `Wrap` body). Prefer [`Self::wrap`] for rmcp registration.
    pub async fn invoke<H, Fut>(&self, params: CallToolRequestParams, handler: H) -> CallToolResult
    where
        H: FnOnce(CallToolRequestParams) -> Fut,
        Fut: Future<Output = CallToolResult>,
    {
        let tool_name = params.name.to_string();
        let payload = extract_payment_from_params(&params);
        let challenge = match self
            .create_payment_required(&tool_name, "Payment Required".into(), payload.as_ref())
            .await
        {
            Ok(required) => required,
            Err(err) => return facilitator_build_error(&err),
        };

        let Some(payload) = payload else {
            return payment_required_tool_result(&challenge);
        };

        let Some(requirements) = self
            .server
            .find_matching_requirements(&challenge.accepts, &payload)
            .cloned()
        else {
            let mut unmatched = challenge;
            unmatched.error = Some("No matching payment requirements found".into());
            return payment_required_tool_result(&unmatched);
        };

        let Ok(flow) = self.server.get_payment_flow(&requirements) else {
            return internal_server_error_result(None);
        };
        let phases = resolve_payment_flow_phases(flow);

        match self.verify_paid(&tool_name, &payload, &requirements).await {
            Err(result) => result,
            Ok(verify_out) => {
                self.after_verify(
                    params,
                    handler,
                    tool_name,
                    payload,
                    requirements,
                    flow,
                    phases.settle_before_handler,
                    verify_out,
                )
                .await
            }
        }
    }

    async fn verify_paid(
        &self,
        tool_name: &str,
        payload: &McpPaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<VerifyPaymentOutcome, CallToolResult> {
        match self.server.verify_payment(payload, requirements).await {
            Ok(out) if out.response.is_valid() => Ok(out),
            Ok(out) => {
                let reason = match out.response {
                    VerifyResponse::Invalid {
                        reason, message, ..
                    } => message.map_or_else(
                        || {
                            reason.map_or_else(
                                || "unexpected_verify_error".to_owned(),
                                |r| r.to_string(),
                            )
                        },
                        |m| m.to_string(),
                    ),
                    _ => "Payment verification failed".into(),
                };
                Err(self
                    .payment_required_result_for(
                        tool_name,
                        format!("Payment verification failed: {reason}"),
                        Some(payload),
                    )
                    .await)
            }
            Err(err) if err.is_transport() => Err(internal_server_error_result(None)),
            Err(err) => Err(self
                .payment_required_result_for(
                    tool_name,
                    format!("Payment verification error: {err}"),
                    Some(payload),
                )
                .await),
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "pipeline state after verify is one sequential hand-off"
    )]
    async fn after_verify<H, Fut>(
        &self,
        params: CallToolRequestParams,
        handler: H,
        tool_name: String,
        payload: McpPaymentPayload,
        requirements: PaymentRequirements,
        flow: PaymentFlowName,
        settle_before_handler: bool,
        verify_out: VerifyPaymentOutcome,
    ) -> CallToolResult
    where
        H: FnOnce(CallToolRequestParams) -> Fut,
        Fut: Future<Output = CallToolResult>,
    {
        let arguments = params.arguments.clone().unwrap_or_default();
        let hook_ctx = ServerHookContext {
            tool_name: tool_name.clone(),
            arguments,
            payment_requirements: requirements.clone(),
            payment_payload: payload.clone(),
        };

        if let Some(ref directive) = verify_out.skip_handler {
            let skip_result = skip_handler_tool_result(directive);
            return self
                .settle_payment_result(
                    &hook_ctx,
                    &payload,
                    &requirements,
                    skip_result,
                    None,
                    &tool_name,
                )
                .await;
        }

        let before_handler = if settle_before_handler {
            match self
                .process_settlement(&payload, &requirements, SettlePhase::BeforeHandler, None)
                .await
            {
                Ok(result) if result.is_success() => Some(CompletedSettlement::new(
                    SettlePhase::BeforeHandler,
                    flow,
                    result,
                    requirements.clone(),
                )),
                Ok(result) => {
                    return self
                        .settlement_failed_result_for(
                            &tool_name,
                            format!("Settlement failed: {}", settle_failure_reason(&result)),
                        )
                        .await;
                }
                Err(err) if err.is_transport() => {
                    return internal_server_error_result(None);
                }
                Err(_) => {
                    return self
                        .settlement_failed_result_for(&tool_name, "Settlement failed")
                        .await;
                }
            }
        } else {
            None
        };

        self.run_handler(
            params,
            handler,
            hook_ctx,
            payload,
            requirements,
            before_handler,
            tool_name,
        )
        .await
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "handler run carries cancel + settle state"
    )]
    async fn run_handler<H, Fut>(
        &self,
        params: CallToolRequestParams,
        handler: H,
        hook_ctx: ServerHookContext,
        payload: McpPaymentPayload,
        requirements: PaymentRequirements,
        before_handler: Option<CompletedSettlement>,
        tool_name: String,
    ) -> CallToolResult
    where
        H: FnOnce(CallToolRequestParams) -> Fut,
        Fut: Future<Output = CallToolResult>,
    {
        let mut cancel = self
            .server
            .cancellation_guard(payload.clone(), requirements.clone());
        if let Some(completed) = before_handler.as_ref() {
            cancel = cancel
                .with_settled_phases([SettlePhase::BeforeHandler])
                .with_before_handler(completed.clone());
        }

        if let Some(ref before) = self.config.hooks.on_before_execution
            && !before(hook_ctx.clone())
        {
            return self
                .payment_required_result_for(
                    &tool_name,
                    "Execution aborted by OnBeforeExecution hook",
                    None,
                )
                .await;
        }

        let result = handler(params).await;

        if let Some(ref after) = self.config.hooks.on_after_execution {
            after(AfterExecutionContext {
                server: hook_ctx.clone(),
                result: result.clone(),
            });
        }

        if result.is_error.unwrap_or(false) {
            let cancel_settlement = cancel
                .cancel(
                    CancelReason::HandlerFailed,
                    Some("tool returned isError"),
                    None,
                )
                .await;
            if let Some(receipt) = build_failure_path_settlement_response(
                cancel_settlement.as_ref(),
                before_handler.as_ref(),
                Some(&payload),
            ) {
                return attach_settle_response(result, &receipt);
            }
            return result;
        }

        self.settle_payment_result(
            &hook_ctx,
            &payload,
            &requirements,
            result,
            before_handler.as_ref(),
            &tool_name,
        )
        .await
    }

    async fn process_settlement(
        &self,
        payload: &McpPaymentPayload,
        requirements: &PaymentRequirements,
        phase: SettlePhase,
        before_handler: Option<&CompletedSettlement>,
    ) -> Result<SettleResponse, FacilitatorError> {
        let flow = self.server.get_payment_flow(requirements).map_err(|err| {
            FacilitatorError::Verification(VerificationError::InvalidFormat(err.to_string()))
        })?;
        let phases = resolve_payment_flow_phases(flow);
        if phase != SettlePhase::BeforeHandler && !phases.settle_after_handler {
            if let Some(before) = before_handler {
                return Ok(before.result.clone());
            }
            return Ok(empty_success_settle(requirements));
        }
        self.server
            .settle_payment(payload, requirements, None, phase)
            .await
    }

    async fn settle_payment_result(
        &self,
        hook_ctx: &ServerHookContext,
        payload: &McpPaymentPayload,
        requirements: &PaymentRequirements,
        result: CallToolResult,
        before_handler: Option<&CompletedSettlement>,
        tool_name: &str,
    ) -> CallToolResult {
        let settle = match self
            .process_settlement(
                payload,
                requirements,
                SettlePhase::AfterHandler,
                before_handler,
            )
            .await
        {
            Ok(settle) if settle.is_success() => settle,
            Ok(settle) => {
                return self
                    .settlement_failed_result_for(
                        tool_name,
                        format!("Settlement failed: {}", settle_failure_reason(&settle)),
                    )
                    .await;
            }
            Err(err) if err.is_transport() => {
                return internal_server_error_result(None);
            }
            Err(_) => {
                return self
                    .settlement_failed_result_for(tool_name, "Settlement failed")
                    .await;
            }
        };

        if let Some(ref after_settle) = self.config.hooks.on_after_settlement {
            after_settle(SettlementContext {
                server: hook_ctx.clone(),
                settlement: settle.clone(),
            });
        }

        attach_settle_response(result, &settle)
    }
}
