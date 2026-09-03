//! Escrow voucher attach and cancel refund for SVM `upto`.

use r402_protocol::{FacilitatorError, PaymentRequirements, VerificationError};
use r402_server::{CancelReason, SettleContext, SettlePhase, VerifiedPaymentCanceledContext};
use serde_json::{Map, Value};
use solana_pubkey::Pubkey;
use solana_signer::Signer;

use crate::upto::VOUCHER_SIGNATURE_FIELD;
use crate::upto::channel::sign_voucher;
use crate::upto::payload::is_upto_svm_payload;

/// Attach a voucher signature on after-handler / cancel settle. Deposit is a no-op.
pub(super) fn attach_voucher<S: Signer + ?Sized>(
    authorizer: &S,
    ctx: &SettleContext,
) -> Result<Option<Map<String, Value>>, FacilitatorError> {
    if ctx.phase == SettlePhase::BeforeHandler {
        return Ok(None);
    }
    if !is_upto_svm_payload(&ctx.payment.payload.payload) {
        return Ok(None);
    }
    let payload = &ctx.payment.payload.payload;
    let authorized = payload
        .get("authorizedSigner")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let authorizer_pk = authorizer.pubkey().to_string();
    if authorized != authorizer_pk {
        return Err(FacilitatorError::Verification(
            VerificationError::InvalidFormat(format!(
                "payload.authorizedSigner {authorized} != configured receiverAuthorizer {authorizer_pk}"
            )),
        ));
    }
    let channel_id: Pubkey = payload
        .get("channelId")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_payload("channelId"))?
        .parse()
        .map_err(|_| invalid_payload("channelId"))?;
    let amount: u64 = ctx
        .payment
        .requirements
        .amount
        .parse()
        .map_err(|_| invalid_payload("settlement amount"))?;
    let expires_at = payload_i64(payload, "expiresAt")?;
    let signature = sign_voucher(authorizer, &channel_id, amount, expires_at).map_err(|err| {
        FacilitatorError::Verification(VerificationError::InvalidSignature(err.to_string()))
    })?;
    let mut extra = Map::new();
    extra.insert(
        VOUCHER_SIGNATURE_FIELD.to_owned(),
        Value::String(signature.to_string()),
    );
    Ok(Some(extra))
}

/// Zero-amount refund requirements for handler failure / abort.
pub(super) fn zero_amount_refund(
    ctx: &VerifiedPaymentCanceledContext,
) -> Option<PaymentRequirements> {
    match ctx.reason {
        CancelReason::HandlerFailed
        | CancelReason::HandlerThrew
        | CancelReason::AfterVerifyAborted => {
            let mut requirements = ctx.payment.requirements.clone();
            requirements.amount = "0".into();
            Some(requirements)
        }
        _ => None,
    }
}

fn payload_i64(payload: &Value, key: &str) -> Result<i64, FacilitatorError> {
    payload
        .get(key)
        .and_then(Value::as_i64)
        .or_else(|| {
            payload
                .get(key)
                .and_then(Value::as_u64)
                .and_then(|v| i64::try_from(v).ok())
        })
        .ok_or_else(|| invalid_payload(key))
}

fn invalid_payload(field: &str) -> FacilitatorError {
    FacilitatorError::Verification(VerificationError::InvalidFormat(format!(
        "invalid upto svm payload: {field}"
    )))
}

#[cfg(test)]
mod tests {
    use r402_server::{PaymentHookContext, WirePaymentPayload};
    use solana_keypair::Keypair;
    use solana_pubkey::Pubkey;
    use solana_signer::Signer;

    use super::*;
    use crate::USDC;
    use crate::upto::channel::{encode_voucher_message, verify_voucher_signature};
    use crate::upto::payload::extra_keys;
    use crate::upto::server::UptoSvmScheme;

    fn scheme() -> (UptoSvmScheme<Keypair>, Keypair) {
        let key = Keypair::new();
        (UptoSvmScheme::new(key.insecure_clone()), key)
    }

    fn requirements(authorizer: &Pubkey, amount: &str) -> PaymentRequirements {
        PaymentRequirements::new(
            "upto".into(),
            "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1"
                .parse()
                .expect("devnet"),
            amount.into(),
            "SysvarRent111111111111111111111111111111111".into(),
            USDC::solana_devnet().address.to_string().into(),
            600,
        )
        .with_extra(serde_json::json!({
            extra_keys::RECEIVER_AUTHORIZER: authorizer.to_string(),
            extra_keys::WITHDRAW_DELAY: 3600,
        }))
    }

    fn payload(authorizer: &Pubkey) -> WirePaymentPayload {
        WirePaymentPayload::new(
            requirements(authorizer, "10000"),
            serde_json::json!({
                "from": "11111111111111111111111111111111",
                "maxAmount": "10000",
                "expiresAt": 1_893_456_000i64,
                "validAfter": 0i64,
                "nonce": "42",
                "openSlot": "341000000",
                "channelId": "SysvarC1ock11111111111111111111111111111111",
                "deposit": "10000",
                "authorizedSigner": authorizer.to_string(),
                "openTransaction": "AQ==",
            }),
        )
    }

    fn settle_ctx(authorizer: &Pubkey, amount: &str, phase: SettlePhase) -> SettleContext {
        SettleContext::new(
            PaymentHookContext::new(payload(authorizer), requirements(authorizer, amount)),
            phase,
        )
    }

    #[tokio::test]
    async fn settle_on_cancel_zero_amount_for_failed_handlers() {
        let (scheme, key) = scheme();
        let requirements = requirements(&key.pubkey(), "10000");
        for reason in [
            CancelReason::HandlerFailed,
            CancelReason::HandlerThrew,
            CancelReason::AfterVerifyAborted,
        ] {
            let ctx = VerifiedPaymentCanceledContext::new(
                PaymentHookContext::new(payload(&key.pubkey()), requirements.clone()),
                reason,
                vec![SettlePhase::BeforeHandler],
            );
            let cancel = r402_server::SchemeNetworkServer::settle_on_cancel(&scheme, &ctx)
                .await
                .expect("refund");
            assert_eq!(cancel.amount.as_str(), "0");
            assert_eq!(cancel.pay_to, requirements.pay_to);
        }
    }

    #[tokio::test]
    async fn enrich_settlement_payload_skips_deposit() {
        let (scheme, key) = scheme();
        let ctx = settle_ctx(&key.pubkey(), "10000", SettlePhase::BeforeHandler);
        assert!(
            r402_server::SchemeNetworkServer::enrich_settlement_payload(&scheme, &ctx)
                .await
                .expect("ok")
                .is_none()
        );
    }

    #[tokio::test]
    async fn enrich_settlement_payload_signs_claim_and_cancel() {
        let (scheme, key) = scheme();
        for (phase, amount) in [
            (SettlePhase::AfterHandler, "1858"),
            (SettlePhase::Cancel, "0"),
        ] {
            let ctx = settle_ctx(&key.pubkey(), amount, phase);
            let fields = r402_server::SchemeNetworkServer::enrich_settlement_payload(&scheme, &ctx)
                .await
                .expect("ok")
                .expect("voucher");
            let sig = fields
                .get(VOUCHER_SIGNATURE_FIELD)
                .and_then(Value::as_str)
                .expect("base58");
            let signature: solana_signature::Signature = sig.parse().expect("sig");
            let channel: Pubkey = "SysvarC1ock11111111111111111111111111111111"
                .parse()
                .expect("channel");
            let message =
                encode_voucher_message(&channel, amount.parse().expect("amt"), 1_893_456_000);
            assert!(verify_voucher_signature(
                &signature,
                &key.pubkey(),
                &message
            ));
        }
    }

    #[tokio::test]
    async fn enrich_settlement_payload_rejects_foreign_authorizer() {
        let (scheme, _) = scheme();
        let other = Keypair::new();
        let ctx = settle_ctx(&other.pubkey(), "1858", SettlePhase::AfterHandler);
        let err = r402_server::SchemeNetworkServer::enrich_settlement_payload(&scheme, &ctx)
            .await
            .expect_err("foreign");
        assert!(err.to_string().contains("authorizedSigner"));
    }

    #[tokio::test]
    async fn enrich_settlement_payload_ignores_foreign_payloads() {
        let (scheme, key) = scheme();
        let ctx = SettleContext::new(
            PaymentHookContext::new(
                WirePaymentPayload::new(
                    requirements(&key.pubkey(), "1858"),
                    serde_json::json!({"transaction": "AQ=="}),
                ),
                requirements(&key.pubkey(), "1858"),
            ),
            SettlePhase::AfterHandler,
        );
        assert!(
            r402_server::SchemeNetworkServer::enrich_settlement_payload(&scheme, &ctx)
                .await
                .expect("ok")
                .is_none()
        );
    }
}
