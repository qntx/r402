//! Server-side [`UptoSvmScheme`]: escrow payment flow and 402 enrich.

mod escrow;

use std::collections::HashMap;
use std::future::Future;
use std::sync::LazyLock;

use r402_protocol::payment::{PaymentRequirements, PriceTag, SupportedResponse, V2};
use r402_protocol::{ChainId, DeployedTokenAmount, FacilitatorError};
use r402_server::{
    PaymentFlowConfig, PaymentFlowName, SchemeNetworkServer, SchemePaymentRequiredContext,
    SettleContext, VerifiedPaymentCanceledContext,
};
use serde_json::{Map, Value};
use solana_signer::Signer;

use super::SolanaUpto;
use super::channel::{default_withdraw_delay, stablecoin_token_program};
use super::payload::{ASSET_TRANSFER_METHOD_CHANNEL, UptoScheme, extra_keys};
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
        std::future::ready(escrow::attach_voucher(&self.receiver_authorizer, ctx))
    }

    fn settle_on_cancel<'a>(
        &'a self,
        ctx: &'a VerifiedPaymentCanceledContext,
    ) -> impl Future<Output = Option<PaymentRequirements>> + Send + 'a {
        std::future::ready(escrow::zero_amount_refund(ctx))
    }

    fn settles_on_cancel(&self) -> bool {
        let _ = self;
        true
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
    ) -> PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let requirements = PaymentRequirements::new(
            UptoScheme.to_string().into(),
            chain_id,
            asset.amount.to_string().into(),
            pay_to.into().to_string().into(),
            asset.token.address.to_string().into(),
            300,
        );
        PriceTag::new(requirements)
    }
}

fn enrich_requirement(
    req: &mut PaymentRequirements,
    supported: &SupportedResponse,
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

fn matching_fee_payer(supported: &SupportedResponse, scheme: &str, network: &str) -> Option<Value> {
    supported.kinds.iter().find_map(|kind| {
        (V2 == kind.x402_version
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

#[cfg(test)]
mod tests {
    use r402_protocol::payment::{PaymentRequired, ResourceInfo, SupportedPaymentKind};
    use r402_server::SchemePaymentRequiredContext;
    use solana_keypair::Keypair;
    use solana_signer::Signer;

    use super::*;
    use crate::USDC;

    fn scheme() -> (UptoSvmScheme<Keypair>, Keypair) {
        let key = Keypair::new();
        (UptoSvmScheme::new(key.insecure_clone()), key)
    }

    fn requirements(authorizer: &solana_pubkey::Pubkey, amount: &str) -> PaymentRequirements {
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
        let payment_required = PaymentRequired::new(ResourceInfo::new("https://example.com/x"));
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

    #[test]
    fn price_tag_is_upto_on_solana() {
        let pay_to: Address = "SysvarRent111111111111111111111111111111111"
            .parse()
            .expect("pay_to");
        let tag = SolanaUpto::price_tag(pay_to, USDC::solana_devnet().amount(10_000));
        assert_eq!(tag.requirements.scheme.as_str(), "upto");
        assert_eq!(tag.requirements.amount.as_str(), "10000");
        assert_eq!(tag.requirements.max_timeout_seconds, 300);
    }
}
