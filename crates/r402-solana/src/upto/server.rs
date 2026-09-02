//! Server-side [`UptoSvmScheme`]: escrow payment flow, 402 enrich, voucher attach.

use std::collections::HashMap;
use std::future::Future;
use std::sync::LazyLock;

use r402_core::chain::{ChainId, DeployedTokenAmount};
use r402_core::error::{FacilitatorError, VerificationError};
use r402_core::resource_server::{
    CancelReason, SchemePaymentRequiredContext, SettleContext, SettlePhase,
    VerifiedPaymentCanceledContext,
};
use r402_core::wire::{self, PaymentRequirements};
use r402_core::{PaymentFlowConfig, PaymentFlowName, SchemeNetworkServer};
use serde_json::{Map, Value};
use solana_pubkey::Pubkey;
use solana_signer::Signer;

use super::payment_channels::sign_voucher;
use super::shared::{default_withdraw_delay, stablecoin_token_program};
use super::types::{ASSET_TRANSFER_METHOD_CHANNEL, UptoScheme, extra_keys, is_upto_svm_payload};
use super::{SolanaUpto, VOUCHER_SIGNATURE_FIELD};
use crate::chain::{Address, SolanaTokenDeployment};

fn upto_svm_payment_flows() -> &'static HashMap<String, PaymentFlowConfig> {
    static FLOWS: LazyLock<HashMap<String, PaymentFlowConfig>> = LazyLock::new(|| {
        HashMap::from([(
            ASSET_TRANSFER_METHOD_CHANNEL.to_owned(),
            PaymentFlowConfig::new(vec![PaymentFlowName::Escrow], PaymentFlowName::Escrow),
        )])
    });
    &FLOWS
}

const DYNAMIC_BLOCKHASH_FIELDS: &[&str] = &[
    extra_keys::RECENT_BLOCKHASH,
    extra_keys::LAST_VALID_BLOCK_HEIGHT,
    extra_keys::RECENT_SLOT,
];

/// SVM server implementation for the `upto` payment scheme.
///
/// Declares the `escrow` payment flow: deposit settle before the handler,
/// voucher claim (or zero-amount cancel) after. HTTP Concurrent/Background
/// are already illegal for escrow.
pub struct UptoSvmScheme<S> {
    receiver_authorizer: S,
    withdraw_delay: Option<u32>,
    rpc_url: Option<String>,
}

impl<S> std::fmt::Debug for UptoSvmScheme<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UptoSvmScheme")
            .field("withdraw_delay", &self.withdraw_delay)
            .field("rpc_url", &self.rpc_url)
            .finish_non_exhaustive()
    }
}

impl<S> UptoSvmScheme<S> {
    /// Construct the server-side upto scheme.
    ///
    /// `receiver_authorizer` is the channel `authorized_signer` and signs
    /// settlement vouchers. It does not need SOL or tokens.
    pub const fn new(receiver_authorizer: S) -> Self {
        Self {
            receiver_authorizer,
            withdraw_delay: None,
            rpc_url: None,
        }
    }

    /// Channel `grace_period`. Defaults to `max(900, maxTimeoutSeconds)`.
    #[must_use]
    pub const fn with_withdraw_delay(mut self, seconds: u32) -> Self {
        self.withdraw_delay = Some(seconds);
        self
    }

    /// RPC endpoint used to embed `recentBlockhash` / `lastValidBlockHeight` /
    /// `recentSlot` in the 402. Best-effort; clients can fetch their own.
    #[must_use]
    pub fn with_rpc_url(mut self, rpc_url: impl Into<String>) -> Self {
        self.rpc_url = Some(rpc_url.into());
        self
    }

    fn withdraw_delay_for(&self, max_timeout_seconds: u64) -> u32 {
        self.withdraw_delay
            .unwrap_or_else(|| default_withdraw_delay(max_timeout_seconds))
    }
}

impl<S: Signer + Send + Sync> SchemeNetworkServer for UptoSvmScheme<S> {
    fn scheme(&self) -> &'static str {
        UptoScheme::VALUE
    }

    fn default_asset_transfer_method(&self) -> &'static str {
        ASSET_TRANSFER_METHOD_CHANNEL
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        upto_svm_payment_flows()
    }

    fn dynamic_extra_fields(&self) -> &[&str] {
        if self.rpc_url.is_some() {
            DYNAMIC_BLOCKHASH_FIELDS
        } else {
            &[]
        }
    }

    async fn enrich_payment_required_response<'a>(
        &'a self,
        ctx: &'a SchemePaymentRequiredContext<'a>,
    ) -> Option<Vec<PaymentRequirements>> {
        Some(self.enrich_accepts(ctx).await)
    }

    fn enrich_settlement_payload<'a>(
        &'a self,
        ctx: &'a SettleContext,
    ) -> impl Future<Output = Result<Option<Map<String, Value>>, FacilitatorError>> + Send + 'a
    {
        std::future::ready(self.attach_voucher(ctx))
    }

    fn settle_on_cancel<'a>(
        &'a self,
        ctx: &'a VerifiedPaymentCanceledContext,
    ) -> impl Future<Output = Option<PaymentRequirements>> + Send + 'a {
        std::future::ready(zero_amount_refund(ctx))
    }
}

impl<S: Signer + Send + Sync> UptoSvmScheme<S> {
    async fn enrich_accepts(
        &self,
        ctx: &SchemePaymentRequiredContext<'_>,
    ) -> Vec<PaymentRequirements> {
        let authorizer = self.receiver_authorizer.pubkey().to_string();
        let mut accepts = ctx.requirements.to_vec();
        for req in &mut accepts {
            if req.scheme.as_str() != UptoScheme::VALUE {
                continue;
            }
            enrich_requirement(
                req,
                ctx.supported,
                &authorizer,
                self.withdraw_delay_for(req.max_timeout_seconds),
            );
            if let Some(rpc_url) = &self.rpc_url {
                attach_blockhash_hints(req, rpc_url).await;
            }
        }
        accepts
    }

    fn attach_voucher(
        &self,
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
        let authorizer = self.receiver_authorizer.pubkey().to_string();
        if authorized != authorizer {
            return Err(FacilitatorError::Verification(
                VerificationError::InvalidFormat(format!(
                    "payload.authorizedSigner {authorized} != configured receiverAuthorizer {authorizer}"
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
        let signature = sign_voucher(&self.receiver_authorizer, &channel_id, amount, expires_at)
            .map_err(|err| {
                FacilitatorError::Verification(VerificationError::InvalidSignature(err.to_string()))
            })?;
        let mut extra = Map::new();
        extra.insert(
            VOUCHER_SIGNATURE_FIELD.to_owned(),
            Value::String(signature.to_string()),
        );
        Ok(Some(extra))
    }
}

impl SolanaUpto {
    /// Creates a price tag for a Solana `upto` payment (authorized maximum).
    #[allow(
        clippy::needless_pass_by_value,
        reason = "mirrors SolanaExact::price_tag"
    )]
    pub fn price_tag<A: Into<Address>>(
        pay_to: A,
        asset: DeployedTokenAmount<u64, SolanaTokenDeployment>,
    ) -> wire::PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let requirements = PaymentRequirements::new(
            UptoScheme.to_string().into(),
            chain_id,
            asset.amount.to_string().into(),
            pay_to.into().to_string().into(),
            asset.token.address.to_string().into(),
            300,
        );
        wire::PriceTag::new(requirements)
    }
}

fn zero_amount_refund(ctx: &VerifiedPaymentCanceledContext) -> Option<PaymentRequirements> {
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

fn enrich_requirement(
    req: &mut PaymentRequirements,
    supported: &wire::SupportedResponse,
    authorizer: &str,
    withdraw_delay: u32,
) {
    let mut extra = req
        .extra
        .take()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    if let Some(fee_payer) =
        matching_fee_payer(supported, UptoScheme::VALUE, &req.network.to_string())
    {
        extra
            .entry(extra_keys::FEE_PAYER.to_owned())
            .or_insert(fee_payer);
    }
    extra.insert(
        extra_keys::RECEIVER_AUTHORIZER.to_owned(),
        Value::String(authorizer.to_owned()),
    );
    extra.insert(
        extra_keys::WITHDRAW_DELAY.to_owned(),
        Value::from(withdraw_delay),
    );
    extra
        .entry(extra_keys::TOKEN_PROGRAM.to_owned())
        .or_insert_with(|| {
            Value::String(stablecoin_token_program(req.asset.as_str(), &req.network).to_string())
        });
    req.extra = Some(Value::Object(extra));
}

fn matching_fee_payer(
    supported: &wire::SupportedResponse,
    scheme: &str,
    network: &str,
) -> Option<Value> {
    supported.kinds.iter().find_map(|kind| {
        (wire::V2 == kind.x402_version
            && kind.scheme.as_str() == scheme
            && kind.network.as_str() == network)
            .then_some(kind.extra.as_ref())
            .flatten()
            .and_then(|extra| extra.get(extra_keys::FEE_PAYER).cloned())
    })
}

async fn attach_blockhash_hints(req: &mut PaymentRequirements, rpc_url: &str) {
    let Ok((blockhash, last_valid, slot)) = fetch_blockhash_hints(rpc_url).await else {
        return;
    };
    let Some(extra) = req.extra.as_mut().and_then(Value::as_object_mut) else {
        return;
    };
    extra.insert(
        extra_keys::RECENT_BLOCKHASH.to_owned(),
        Value::String(blockhash),
    );
    extra.insert(
        extra_keys::LAST_VALID_BLOCK_HEIGHT.to_owned(),
        Value::String(last_valid.to_string()),
    );
    extra.insert(
        extra_keys::RECENT_SLOT.to_owned(),
        Value::String(slot.to_string()),
    );
}

async fn fetch_blockhash_hints(rpc_url: &str) -> Result<(String, u64, u64), FacilitatorError> {
    use solana_client::nonblocking::rpc_client::RpcClient;
    use solana_commitment_config::{CommitmentConfig, CommitmentLevel};

    let client = RpcClient::new(rpc_url.to_owned());
    let commitment = CommitmentConfig {
        commitment: CommitmentLevel::Finalized,
    };
    let (blockhash, last_valid) = client
        .get_latest_blockhash_with_commitment(commitment)
        .await
        .map_err(|err| FacilitatorError::Onchain(err.to_string()))?;
    let slot = client
        .get_slot_with_commitment(commitment)
        .await
        .map_err(|err| FacilitatorError::Onchain(err.to_string()))?;
    Ok((blockhash.to_string(), last_valid, slot))
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
    use std::future::Future;
    use std::sync::Arc;

    use r402_core::chain::ChainIdPattern;
    use r402_core::facilitator::Facilitator;
    use r402_core::resource_server::{
        PaymentHookContext, PaymentRequiredBuildContext, ResourceServer, SettleContext,
        SettlePhase, VerifiedPaymentCanceledContext, WirePaymentPayload,
    };
    use r402_core::wire::{
        Extensions, ResourceInfo, SettleRequest, SettleResponse, SupportedPaymentKind,
        SupportedResponse, VerifyRequest, VerifyResponse,
    };
    use solana_keypair::Keypair;
    use solana_pubkey::Pubkey;
    use solana_signer::Signer;

    use super::*;
    use crate::USDC;
    use crate::upto::payment_channels::{encode_voucher_message, verify_voucher_signature};

    struct NoopFacilitator;

    impl Facilitator for NoopFacilitator {
        fn verify(
            &self,
            _request: VerifyRequest,
        ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(VerifyResponse::valid("payer")))
        }

        fn settle(
            &self,
            _request: SettleRequest,
        ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(SettleResponse::Success {
                payer: "payer".into(),
                transaction: "sig".into(),
                network: "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1".into(),
                amount: Some("10000".into()),
                extensions: Extensions::new(),
                extra: None,
            }))
        }

        fn supported(
            &self,
        ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(SupportedResponse::default()))
        }
    }

    fn scheme() -> (UptoSvmScheme<Keypair>, Keypair) {
        let key = Keypair::new();
        (UptoSvmScheme::new(key.insecure_clone()), key)
    }

    fn requirements(authorizer: &Pubkey, amount: &str) -> PaymentRequirements {
        PaymentRequirements::new(
            "upto".into(),
            "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1".parse().unwrap(),
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

    #[test]
    fn payment_flows_declare_escrow_channel_only() {
        let (scheme, _) = scheme();
        assert_eq!(scheme.scheme(), "upto");
        assert_eq!(
            scheme.default_asset_transfer_method(),
            ASSET_TRANSFER_METHOD_CHANNEL
        );
        let row = scheme
            .payment_flows()
            .get(ASSET_TRANSFER_METHOD_CHANNEL)
            .expect("channel ATM");
        assert_eq!(
            *row,
            PaymentFlowConfig::new(vec![PaymentFlowName::Escrow], PaymentFlowName::Escrow)
        );
        assert!(scheme.dynamic_extra_fields().is_empty());
    }

    #[tokio::test]
    async fn create_payment_required_writes_escrow_payment_flow() {
        let (scheme, _) = scheme();
        let rs = ResourceServer::new(Arc::new(NoopFacilitator))
            .with_scheme(ChainIdPattern::wildcard("solana"), scheme);
        let req = PaymentRequirements::new(
            "upto".into(),
            "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1".parse().unwrap(),
            "10000".into(),
            "SysvarRent111111111111111111111111111111111".into(),
            USDC::solana_devnet().address.to_string().into(),
            600,
        );
        let built = rs
            .create_payment_required_response(
                vec![req],
                PaymentRequiredBuildContext {
                    resource: ResourceInfo::new("https://example.com/upto"),
                    error: None,
                    extensions: Extensions::new(),
                    supported: SupportedResponse::default(),
                    payment_payload: None,
                },
            )
            .await
            .unwrap();
        let extra = built
            .accepts
            .first()
            .and_then(|accept| accept.extra.as_ref())
            .expect("upto 402 extra");
        assert_eq!(
            extra.get("paymentFlow").and_then(Value::as_str),
            Some("escrow")
        );
        assert!(extra.get("assetTransferMethod").is_none());
    }

    #[test]
    fn dynamic_extra_fields_only_when_rpc_configured() {
        let (scheme, _) = scheme();
        let with_rpc = scheme.with_rpc_url("https://api.devnet.solana.com");
        assert_eq!(
            with_rpc.dynamic_extra_fields(),
            &[
                extra_keys::RECENT_BLOCKHASH,
                extra_keys::LAST_VALID_BLOCK_HEIGHT,
                extra_keys::RECENT_SLOT
            ]
        );
    }

    #[tokio::test]
    async fn enrich_folds_fee_payer_authorizer_and_token_program() {
        let (scheme, key) = scheme();
        let fee_payer = "SysvarC1ock11111111111111111111111111111111";
        let supported = SupportedResponse::new().with_kinds(vec![
            SupportedPaymentKind::new(2, "upto", "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1")
                .with_extra(serde_json::json!({ extra_keys::FEE_PAYER: fee_payer })),
        ]);
        let accept = requirements(&key.pubkey(), "10000");
        let payment_required =
            wire::PaymentRequired::new(ResourceInfo::new("https://example.com/x"));
        let ctx = SchemePaymentRequiredContext::new(
            std::slice::from_ref(&accept),
            &payment_required.resource,
            &payment_required,
            &supported,
        );
        let enriched = scheme
            .enrich_payment_required_response(&ctx)
            .await
            .expect("enrich writes extra");
        let extra = enriched
            .first()
            .and_then(|req| req.extra.as_ref())
            .and_then(Value::as_object)
            .expect("extra");
        assert_eq!(
            extra.get(extra_keys::FEE_PAYER).and_then(Value::as_str),
            Some(fee_payer)
        );
        assert_eq!(
            extra
                .get(extra_keys::RECEIVER_AUTHORIZER)
                .and_then(Value::as_str),
            Some(key.pubkey().to_string().as_str())
        );
        assert_eq!(
            extra
                .get(extra_keys::WITHDRAW_DELAY)
                .and_then(Value::as_u64),
            Some(900)
        );
        assert_eq!(
            extra.get(extra_keys::TOKEN_PROGRAM).and_then(Value::as_str),
            Some(crate::chain::TOKEN_PROGRAM_ID.to_string().as_str())
        );
        assert!(!extra.contains_key(extra_keys::RECENT_BLOCKHASH));
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
            let cancel = scheme.settle_on_cancel(&ctx).await.expect("refund");
            assert_eq!(cancel.amount.as_str(), "0");
            assert_eq!(cancel.pay_to, requirements.pay_to);
        }
    }

    #[tokio::test]
    async fn enrich_settlement_payload_skips_deposit() {
        let (scheme, key) = scheme();
        let ctx = settle_ctx(&key.pubkey(), "10000", SettlePhase::BeforeHandler);
        assert!(
            scheme
                .enrich_settlement_payload(&ctx)
                .await
                .unwrap()
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
            let fields = scheme
                .enrich_settlement_payload(&ctx)
                .await
                .unwrap()
                .expect("voucher");
            let sig = fields
                .get(VOUCHER_SIGNATURE_FIELD)
                .and_then(Value::as_str)
                .expect("base58");
            let signature: solana_signature::Signature = sig.parse().unwrap();
            let channel: Pubkey = "SysvarC1ock11111111111111111111111111111111"
                .parse()
                .unwrap();
            let message = encode_voucher_message(&channel, amount.parse().unwrap(), 1_893_456_000);
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
        let err = scheme.enrich_settlement_payload(&ctx).await.unwrap_err();
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
            scheme
                .enrich_settlement_payload(&ctx)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn price_tag_is_upto_on_solana() {
        let pay_to: Address = "SysvarRent111111111111111111111111111111111"
            .parse()
            .unwrap();
        let tag = SolanaUpto::price_tag(pay_to, USDC::solana_devnet().amount(10_000));
        assert_eq!(tag.requirements.scheme.as_str(), "upto");
        assert_eq!(tag.requirements.amount.as_str(), "10000");
        assert_eq!(tag.requirements.max_timeout_seconds, 300);
    }
}
