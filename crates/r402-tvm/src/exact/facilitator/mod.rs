//! Facilitator-side payment verification and settlement for the TON exact scheme.

mod settle;
mod verify;

use std::collections::HashMap;
use std::future::Future;
use std::time::{SystemTime, UNIX_EPOCH};

use compact_str::CompactString;
use r402_core::cache::SettlementCache;
use r402_core::chain::ChainProvider;
use r402_core::error::FacilitatorError;
use r402_core::facilitator::{DynFacilitator, Facilitator};
use r402_core::scheme::{SchemeBuilder, SchemeId};
use r402_core::wire;
use serde::Deserialize;
pub use settle::settle_request;
pub use verify::{
    VerifiedSettlement, finalized_trace_tx_hash, verify_finalized_trace_settlement,
    verify_request_json, verify_response_json,
};

#[cfg(test)]
mod tests;

use crate::chain::TvmChainProvider;
use crate::exact::settlement_batcher::SettlementBatcher;
use crate::exact::{ExactScheme, TvmExact, TvmExtra};

/// Optional JSON config for [`TvmExact`] [`SchemeBuilder`].
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TvmExactFacilitatorConfig {
    /// Idle flush interval for the Highload batcher.
    #[serde(default)]
    pub batch_flush_interval_seconds: Option<u64>,
    /// Queue length that triggers an immediate flush.
    #[serde(default)]
    pub batch_flush_size: Option<usize>,
    /// Trace confirmation timeout.
    #[serde(default)]
    pub confirmation_timeout_seconds: Option<u64>,
}

/// Facilitator for TON exact scheme payments.
pub struct TvmExactFacilitator<P> {
    provider: P,
    settlement_cache: SettlementCache,
    batcher: SettlementBatcher,
}

impl TvmExactFacilitator<TvmChainProvider> {
    /// Creates a facilitator with the default in-memory settlement cache.
    #[must_use]
    pub fn new(provider: TvmChainProvider, config: TvmExactFacilitatorConfig) -> Self {
        Self::with_settlement_cache(provider, config, SettlementCache::new())
    }

    /// Creates a facilitator with a shared [`SettlementCache`].
    #[must_use]
    pub fn with_settlement_cache(
        provider: TvmChainProvider,
        config: TvmExactFacilitatorConfig,
        settlement_cache: SettlementCache,
    ) -> Self {
        let batcher = SettlementBatcher::new(
            provider.clone(),
            config.batch_flush_interval_seconds,
            config.batch_flush_size,
            config.confirmation_timeout_seconds,
            |trace, settlement| finalized_trace_tx_hash(trace, settlement),
        );
        Self {
            provider,
            settlement_cache,
            batcher,
        }
    }
}

impl<P> std::fmt::Debug for TvmExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TvmExactFacilitator")
            .finish_non_exhaustive()
    }
}

impl SchemeBuilder<TvmChainProvider> for TvmExact {
    fn build(
        &self,
        provider: TvmChainProvider,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        let parsed = config
            .map(serde_json::from_value::<TvmExactFacilitatorConfig>)
            .transpose()?
            .unwrap_or_default();
        Ok(Box::new(TvmExactFacilitator::new(provider, parsed)))
    }
}

impl SchemeBuilder<&TvmChainProvider> for TvmExact {
    fn build(
        &self,
        provider: &TvmChainProvider,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        SchemeBuilder::<TvmChainProvider>::build(self, provider.clone(), config)
    }
}

impl Facilitator for TvmExactFacilitator<TvmChainProvider> {
    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_tvm::exact::verify", skip_all)
    )]
    async fn verify(
        &self,
        request: wire::VerifyRequest,
    ) -> Result<wire::VerifyResponse, FacilitatorError> {
        let json = request.into_json();
        let now = now_unix();
        let provider = self.provider.clone();
        Ok(verify_response_json(
            &self.provider,
            &self.provider.signer_addresses(),
            &json,
            now,
            async move |relay| {
                let boc = provider
                    .build_relay_external_boc_batch(std::slice::from_ref(relay), true)
                    .await
                    .map_err(|e| e.to_string())?;
                provider
                    .emulate_external_message(&boc)
                    .await
                    .map_err(|e| e.to_string())
            },
        )
        .await)
    }

    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_tvm::exact::settle", skip_all)
    )]
    async fn settle(
        &self,
        request: wire::SettleRequest,
    ) -> Result<wire::SettleResponse, FacilitatorError> {
        let json = request.into_json();
        let now = now_unix();
        let provider = self.provider.clone();
        Ok(settle_request(
            &self.provider,
            &self.provider.signer_addresses(),
            &self.settlement_cache,
            &self.batcher,
            &json,
            now,
            async move |relay| {
                let boc = provider
                    .build_relay_external_boc_batch(std::slice::from_ref(relay), true)
                    .await
                    .map_err(|e| e.to_string())?;
                provider
                    .emulate_external_message(&boc)
                    .await
                    .map_err(|e| e.to_string())
            },
        )
        .await)
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<wire::SupportedResponse, FacilitatorError>> + Send {
        let chain_id = self.provider.chain_id();
        let extra = serde_json::to_value(TvmExtra::sponsored()).ok();
        let kinds = vec![
            wire::SupportedPaymentKind::new(
                wire::V2.into(),
                ExactScheme.to_string(),
                chain_id.to_string(),
            )
            .with_optional_extra(extra),
        ];
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::with_capacity(1);
        let _ = signers.insert(
            TvmExact.caip_family().into(),
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

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
