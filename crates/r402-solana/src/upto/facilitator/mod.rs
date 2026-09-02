//! Facilitator-side verify and settle for Solana `upto` payment-channel escrow.
#![allow(
    unreachable_pub,
    reason = "submodules are crate-private; items are pub for intra-facilitator use"
)]

mod config;
mod settle;
mod storage;
mod verify;

use std::collections::HashMap;
use std::sync::Arc;

use compact_str::CompactString;
pub use config::{DEFAULT_MAX_CHANNEL_LIFETIME_SECS, SolanaUptoFacilitatorConfig};
use r402_core::chain::ChainProvider;
use r402_core::error::{FacilitatorError, VerificationError};
use r402_core::facilitator::{DynFacilitator, Facilitator};
use r402_core::pending_settlement::{InMemoryPendingSettlementStore, PendingSettlementStore};
use r402_core::scheme::{SchemeBuilder, SchemeId};
use r402_core::wire;
pub use storage::{InMemoryChannelStorage, UptoChannelRecord, UptoChannelStorage};
use verify::validate_open_authorization;

use super::SolanaUpto;
use super::error::codes;
use super::types::{UptoScheme, UptoSvmPayload, extra_keys, is_upto_svm_payload};
use crate::chain::provider::SolanaChainProviderLike;
use crate::exact::SupportedPaymentKindExtra;

/// Facilitator for Solana `upto` payments.
///
/// `/verify` is a read-only preflight of the channel `open`. `/settle`
/// without `voucherSignature` and with `amount == maxAmount` deposits
/// (broadcasts `open`); `/settle` with a voucher claims.
pub struct SolanaUptoFacilitator<P> {
    provider: P,
    config: SolanaUptoFacilitatorConfig,
    storage: Arc<dyn UptoChannelStorage>,
    pending: Arc<dyn PendingSettlementStore>,
    inflight: settle::InflightSettlements,
}

impl<P> SolanaUptoFacilitator<P> {
    /// Creates a facilitator with in-memory channel storage and pending store.
    pub fn new(provider: P, config: SolanaUptoFacilitatorConfig) -> Self {
        Self {
            provider,
            config,
            storage: Arc::new(InMemoryChannelStorage::new()),
            pending: Arc::new(InMemoryPendingSettlementStore::new()),
            inflight: settle::InflightSettlements::default(),
        }
    }

    /// Override channel storage (durable index for rent cleanup).
    #[must_use]
    pub fn with_storage(mut self, storage: Arc<dyn UptoChannelStorage>) -> Self {
        self.storage = storage;
        self
    }

    /// Override pending-settlement store.
    #[must_use]
    pub fn with_pending_store(mut self, pending: Arc<dyn PendingSettlementStore>) -> Self {
        self.pending = pending;
        self
    }
}

impl<P> std::fmt::Debug for SolanaUptoFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SolanaUptoFacilitator")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl<P> SchemeBuilder<P> for SolanaUpto
where
    P: SolanaChainProviderLike + ChainProvider + Send + Sync + 'static,
{
    fn build(
        &self,
        provider: P,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        let config = config
            .map(serde_json::from_value::<SolanaUptoFacilitatorConfig>)
            .transpose()?
            .unwrap_or_default();
        Ok(Box::new(SolanaUptoFacilitator::new(provider, config)))
    }
}

impl<P> Facilitator for SolanaUptoFacilitator<P>
where
    P: SolanaChainProviderLike + ChainProvider + Send + Sync,
{
    async fn verify(
        &self,
        request: wire::VerifyRequest,
    ) -> Result<wire::VerifyResponse, FacilitatorError> {
        let (payload, requirements, accepted_scheme, accepted_network) =
            decode_request(&request.into_json())?;
        match validate_open_authorization(
            &self.provider,
            &payload,
            &requirements,
            &accepted_scheme,
            &accepted_network,
            true,
            &self.config,
        ) {
            Ok(auth) => Ok(wire::VerifyResponse::valid(auth.payload.from)),
            Err(failure) => Ok(failure.into_verify()),
        }
    }

    async fn settle(
        &self,
        request: wire::SettleRequest,
    ) -> Result<wire::SettleResponse, FacilitatorError> {
        let json = request.into_json();
        let (payload, requirements, accepted_scheme, accepted_network) = decode_request(&json)?;
        let network = requirements.network.to_string();
        if accepted_scheme != UptoScheme::VALUE || requirements.scheme.as_str() != UptoScheme::VALUE
        {
            return Ok(verify::UptoFailure::new(
                "unsupported_scheme",
                &payload.from,
                "unsupported_scheme",
            )
            .into_settle(network));
        }
        if accepted_network != network {
            return Ok(verify::UptoFailure::new(
                "network_mismatch",
                &payload.from,
                "network_mismatch",
            )
            .into_settle(network));
        }
        let actual = payload_amount(&requirements.amount, &payload)?;
        let max_amount = payload_amount(&payload.max_amount, &payload)?;
        if actual > max_amount {
            return Ok(verify::UptoFailure::new(
                codes::SETTLEMENT_EXCEEDS_AMOUNT,
                &payload.from,
                "settlement exceeds authorized maximum",
            )
            .into_settle(network));
        }
        if payload.voucher_signature.is_some() {
            return settle::settle_claim(
                &self.provider,
                &payload,
                &requirements,
                actual,
                &self.config,
                self.storage.as_ref(),
                self.pending.as_ref(),
                &self.inflight,
            )
            .await;
        }
        if actual == max_amount {
            let auth = match validate_open_authorization(
                &self.provider,
                &payload,
                &requirements,
                &accepted_scheme,
                &accepted_network,
                true,
                &self.config,
            ) {
                Ok(auth) => auth,
                Err(failure) => return Ok(failure.into_settle(network)),
            };
            return settle::settle_deposit(
                &self.provider,
                &payload,
                &requirements,
                auth,
                &self.config,
                self.storage.as_ref(),
                self.pending.as_ref(),
                &self.inflight,
            )
            .await;
        }
        Ok(verify::UptoFailure::new(
            codes::MISSING_VOUCHER,
            &payload.from,
            "partial charge requires voucherSignature",
        )
        .into_settle(network))
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<wire::SupportedResponse, FacilitatorError>> + Send {
        let chain_id = self.provider.chain_id();
        let extra = serde_json::to_value(SupportedPaymentKindExtra {
            fee_payer: self.provider.fee_payer(),
            memo: None,
        })
        .ok();
        let kinds = vec![
            wire::SupportedPaymentKind::new(
                wire::V2.into(),
                UptoScheme.to_string(),
                chain_id.to_string(),
            )
            .with_optional_extra(extra),
        ];
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::with_capacity(1);
        let _ = signers.insert(
            SolanaUpto.caip_family().into(),
            self.provider
                .signer_addresses()
                .into_iter()
                .map(CompactString::from)
                .collect(),
        );
        std::future::ready(Ok(wire::SupportedResponse::new()
            .with_kinds(kinds)
            .with_signers(signers)))
    }
}

fn decode_request(
    json: &serde_json::Value,
) -> Result<(UptoSvmPayload, wire::PaymentRequirements, String, String), FacilitatorError> {
    let payload_val = json
        .pointer("/paymentPayload/payload")
        .cloned()
        .ok_or_else(|| invalid("missing paymentPayload.payload"))?;
    if !is_upto_svm_payload(&payload_val) {
        return Err(invalid("unsupported_payload_type"));
    }
    let payload: UptoSvmPayload = serde_json::from_value(payload_val).map_err(|err| {
        FacilitatorError::Verification(VerificationError::InvalidFormat(err.to_string()))
    })?;
    let requirements: wire::PaymentRequirements = serde_json::from_value(
        json.pointer("/paymentRequirements")
            .cloned()
            .ok_or_else(|| invalid("missing paymentRequirements"))?,
    )
    .map_err(|err| {
        FacilitatorError::Verification(VerificationError::InvalidFormat(err.to_string()))
    })?;
    let accepted_scheme = json
        .pointer("/paymentPayload/accepted/scheme")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let accepted_network = json
        .pointer("/paymentPayload/accepted/network")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let _ = extra_keys::FEE_PAYER;
    Ok((payload, requirements, accepted_scheme, accepted_network))
}

fn payload_amount(raw: &str, payload: &UptoSvmPayload) -> Result<u64, FacilitatorError> {
    raw.parse().map_err(|_| {
        verify::UptoFailure::new(
            codes::PAYLOAD_AMOUNT,
            &payload.from,
            "not an unsigned integer",
        )
        .into_error()
    })
}

fn invalid(message: &str) -> FacilitatorError {
    FacilitatorError::Verification(VerificationError::InvalidFormat(message.into()))
}

use std::future::Future;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_spec_ceilings() {
        let config = SolanaUptoFacilitatorConfig::default();
        assert_eq!(config.max_channel_lifetime_secs, 3_600);
        assert_eq!(config.max_compute_units, 400_000);
    }
}
