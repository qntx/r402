//! Facilitator-side payment verification and settlement for the TON exact scheme.

mod batcher;
mod settle;
mod verify;

use std::collections::HashMap;
use std::future::Future;
use std::time::{SystemTime, UNIX_EPOCH};

pub use batcher::SettlementBatcher;
use compact_str::CompactString;
use r402_facilitator::{Facilitator, SettlementCache};
use r402_protocol::error::FacilitatorError;
use r402_protocol::network::ChainProvider;
use r402_protocol::payment::{
    SettleRequest, SettleResponse, SupportedPaymentKind, SupportedResponse, V2, VerifyRequest,
    VerifyResponse,
};
use r402_protocol::scheme::SchemeId;
use serde::Deserialize;
pub use settle::settle_request;
pub use verify::{
    VerifiedSettlement, finalized_trace_tx_hash, verify_finalized_trace_settlement,
    verify_request_json, verify_response_json,
};

use crate::chain::TvmChainProvider;
use crate::exact::{ExactScheme, TvmExact, TvmExtra};

/// Optional JSON config for [`TvmExactFacilitator`].
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
pub struct TvmExactFacilitator {
    provider: TvmChainProvider,
    settlement_cache: SettlementCache,
    batcher: SettlementBatcher,
}

impl TvmExactFacilitator {
    /// Constructs a facilitator from a chain provider.
    ///
    /// Uses default batcher intervals and a default [`SettlementCache`].
    ///
    /// # Errors
    ///
    /// Currently infallible. [`Result`] so `try_new(provider)?` compiles.
    #[allow(
        clippy::unnecessary_wraps,
        reason = "Result so try_new(provider)? compiles"
    )]
    pub fn try_new(provider: TvmChainProvider) -> Result<Self, FacilitatorError> {
        Ok(Self::new(provider, TvmExactFacilitatorConfig::default()))
    }

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

impl std::fmt::Debug for TvmExactFacilitator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TvmExactFacilitator")
            .finish_non_exhaustive()
    }
}

impl Facilitator for TvmExactFacilitator {
    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_tvm::exact::verify", skip_all)
    )]
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
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
    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
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
    ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
        let chain_id = self.provider.chain_id();
        let extra = serde_json::to_value(TvmExtra::sponsored()).ok();
        let kinds = vec![
            SupportedPaymentKind::new(V2.into(), ExactScheme.to_string(), chain_id.to_string())
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
        std::future::ready(Ok(SupportedResponse::new()
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
