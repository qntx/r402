//! Verified-payment cancellation and failure-path receipts.

use std::sync::atomic::{AtomicBool, Ordering};

use compact_str::CompactString;
use r402_protocol::payment::{PaymentRequirements, SettleResponse};
use serde_json::Value;

use crate::hooks::{
    CancelReason, PaymentHookContext, VerifiedPaymentCanceledContext, WirePaymentPayload,
};
use crate::payment_flow::SettlePhase;
use crate::resource::ResourceServer;
use crate::settle::CompletedSettlement;

impl ResourceServer {
    /// Fires `on_verified_payment_canceled` for every registered hook.
    pub async fn notify_verified_payment_canceled(
        &self,
        payload: &WirePaymentPayload,
        requirements: &PaymentRequirements,
        reason: CancelReason,
        error: Option<&str>,
        response_status: Option<u16>,
    ) {
        if self.hooks.is_empty() {
            return;
        }
        let ctx = VerifiedPaymentCanceledContext {
            payment: PaymentHookContext {
                payload: payload.clone(),
                requirements: requirements.clone(),
            },
            reason,
            error: error.map(str::to_owned),
            response_status,
            settled_phases: Vec::new(),
        };
        for hook in &self.hooks {
            hook.on_verified_payment_canceled(&ctx).await;
        }
    }

    /// Builds a one-shot cancellation dispatcher (first call wins).
    #[must_use]
    pub fn cancellation_guard(
        &self,
        payload: WirePaymentPayload,
        requirements: PaymentRequirements,
    ) -> CancellationGuard {
        CancellationGuard {
            server: self.clone(),
            payload,
            requirements,
            fired: AtomicBool::new(false),
            settled_phases: Vec::new(),
            before_handler: None,
        }
    }

    async fn settle_canceled_payment(
        &self,
        ctx: &VerifiedPaymentCanceledContext,
    ) -> Option<SettleResponse> {
        if !ctx.settled_phases.contains(&SettlePhase::BeforeHandler) {
            return None;
        }
        let scheme = self.registered_scheme(
            ctx.payment.requirements.scheme.as_str(),
            &ctx.payment.requirements.network,
        )?;
        let cancel_requirements = scheme.settle_on_cancel(ctx).await?;
        match self
            .settle_payment(
                &ctx.payment.payload,
                &cancel_requirements,
                None,
                SettlePhase::Cancel,
                ctx.payment
                    .payload
                    .resource
                    .as_ref()
                    .map(|r| r.url.as_str()),
                None,
            )
            .await
        {
            Ok(response) => Some(response),
            Err(error) => SettleResponse::from_facilitator_error(
                &error,
                ctx.payment.requirements.network.to_string(),
                "",
            ),
        }
    }
}

/// Ensures `on_verified_payment_canceled` runs at most once for a payment.
#[derive(Debug)]
pub struct CancellationGuard {
    server: ResourceServer,
    payload: WirePaymentPayload,
    requirements: PaymentRequirements,
    fired: AtomicBool,
    settled_phases: Vec<SettlePhase>,
    before_handler: Option<CompletedSettlement>,
}

impl CancellationGuard {
    /// Records settle phases already completed before the handler.
    #[must_use]
    pub fn with_settled_phases(mut self, phases: impl IntoIterator<Item = SettlePhase>) -> Self {
        self.settled_phases = phases.into_iter().collect();
        self
    }

    /// Stores the before-handler settle receipt for failure-path echo.
    #[must_use]
    pub fn with_before_handler(mut self, settlement: CompletedSettlement) -> Self {
        self.before_handler = Some(settlement);
        self
    }

    /// Settle phases recorded on this guard.
    #[must_use]
    pub const fn settled_phases(&self) -> &[SettlePhase] {
        self.settled_phases.as_slice()
    }

    /// Before-handler settle receipt, when one was stored.
    #[must_use]
    pub const fn before_handler(&self) -> Option<&CompletedSettlement> {
        self.before_handler.as_ref()
    }

    /// Fires cancel hooks if not already fired.
    ///
    /// When [`SettlePhase::BeforeHandler`] is in [`Self::settled_phases`] and
    /// the matched scheme's `settle_on_cancel` returns requirements, settles
    /// at [`SettlePhase::Cancel`].
    pub async fn cancel(
        &self,
        reason: CancelReason,
        error: Option<&str>,
        response_status: Option<u16>,
    ) -> Option<SettleResponse> {
        if self
            .fired
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return None;
        }
        let ctx = VerifiedPaymentCanceledContext {
            payment: PaymentHookContext {
                payload: self.payload.clone(),
                requirements: self.requirements.clone(),
            },
            reason,
            error: error.map(str::to_owned),
            response_status,
            settled_phases: self.settled_phases.clone(),
        };
        for hook in &self.server.hooks {
            hook.on_verified_payment_canceled(&ctx).await;
        }
        self.server.settle_canceled_payment(&ctx).await
    }

    /// Returns `true` when cancel has already been dispatched.
    #[must_use]
    pub fn has_fired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
    }
}

/// Settlement receipt attached when the resource handler fails after verify.
///
/// Preference: successful cancel settle, then failed cancel with deposit-recovery
/// extras, then before-handler echo, else `None`.
#[must_use]
pub fn build_failure_path_settlement_response(
    cancel_settlement: Option<&SettleResponse>,
    before_handler: Option<&CompletedSettlement>,
    payment_payload: Option<&WirePaymentPayload>,
) -> Option<SettleResponse> {
    if let Some(cancel) = cancel_settlement {
        if cancel.is_success() {
            return Some(cancel.clone());
        }
        return Some(build_failed_cancel_receipt(
            cancel,
            before_handler,
            payment_payload,
        ));
    }
    before_handler.map(|completed| completed.result.clone())
}

/// Failed cancel receipt with deposit-recovery facts in `extra`.
fn build_failed_cancel_receipt(
    cancel: &SettleResponse,
    before_handler: Option<&CompletedSettlement>,
    payment_payload: Option<&WirePaymentPayload>,
) -> SettleResponse {
    let SettleResponse::Failure {
        reason,
        message,
        payer,
        network,
        extensions,
        extension_responses,
        extra: cancel_extra,
        ..
    } = cancel
    else {
        return cancel.clone();
    };

    let mut extra_map = match cancel_extra {
        Some(Value::Object(map)) => map.clone(),
        _ => serde_json::Map::new(),
    };
    if let Some(before) = before_handler {
        extra_map.insert(
            "depositTransaction".into(),
            Value::String(settle_response_transaction(&before.result).to_owned()),
        );
        extra_map.insert(
            "depositAmount".into(),
            Value::String(
                settle_response_amount(&before.result)
                    .unwrap_or("")
                    .to_owned(),
            ),
        );
    }
    if let Some(payload) = payment_payload
        && let Some(channel_id) = payload
            .payload
            .get("channelId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
    {
        extra_map.insert("channelId".into(), Value::String(channel_id.to_owned()));
    }
    let merged_extra = if extra_map.is_empty() {
        None
    } else {
        Some(Value::Object(extra_map))
    };
    SettleResponse::Failure {
        reason: reason.clone(),
        message: message.clone(),
        payer: payer.clone(),
        transaction: CompactString::default(),
        network: network.clone(),
        extensions: extensions.clone(),
        extension_responses: extension_responses.clone(),
        extra: merged_extra,
    }
}

fn settle_response_transaction(response: &SettleResponse) -> &str {
    match response {
        SettleResponse::Success { transaction, .. }
        | SettleResponse::Failure { transaction, .. } => transaction.as_str(),
        _ => "",
    }
}

fn settle_response_amount(response: &SettleResponse) -> Option<&str> {
    match response {
        SettleResponse::Success { amount, .. } => amount.as_deref(),
        SettleResponse::Failure { .. } | _ => None,
    }
}
