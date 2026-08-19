//! Facilitator-side payment verification and settlement for the XRPL exact scheme.

mod settle;
mod verify;

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use compact_str::CompactString;
use r402_core::cache::{DEFAULT_SETTLEMENT_CAPACITY, SettlementCache};
use r402_core::chain::ChainProvider;
use r402_core::error::FacilitatorError;
use r402_core::facilitator::{DynFacilitator, Facilitator};
use r402_core::scheme::{SchemeBuilder, SchemeId};
use r402_core::wire;
use serde::Deserialize;
pub use settle::settle_request;
pub use verify::{default_max_fee_drops, verify_request_json};

#[cfg(test)]
mod tests;

use crate::SETTLEMENT_TTL_MS;
use crate::chain::XrplChainProvider;
use crate::exact::{ExactScheme, XrplExact};

/// Optional JSON config for [`XrplExact`] [`SchemeBuilder`].
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XrplExactFacilitatorConfig {
    /// Maximum accepted fee in drops.
    ///
    /// When omitted, [`XrplExact`] [`SchemeBuilder`] uses
    /// [`crate::chain::XrplChainProvider::max_fee_drops`].
    #[serde(default)]
    pub max_fee_drops: Option<u64>,
}

/// Facilitator for XRPL exact scheme payments.
pub struct XrplExactFacilitator<P> {
    provider: P,
    max_fee_drops: u64,
    settlement_cache: SettlementCache,
}

impl<P> XrplExactFacilitator<P> {
    /// Creates a facilitator with a settlement cache covering the landable window floor.
    pub fn new(provider: P, max_fee_drops: u64) -> Self {
        Self::with_settlement_cache(
            provider,
            max_fee_drops,
            SettlementCache::with_params(
                Duration::from_millis(SETTLEMENT_TTL_MS),
                DEFAULT_SETTLEMENT_CAPACITY,
            ),
        )
    }

    /// Creates a facilitator with a shared [`SettlementCache`].
    pub const fn with_settlement_cache(
        provider: P,
        max_fee_drops: u64,
        settlement_cache: SettlementCache,
    ) -> Self {
        Self {
            provider,
            max_fee_drops,
            settlement_cache,
        }
    }
}

impl<P> std::fmt::Debug for XrplExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XrplExactFacilitator")
            .field("max_fee_drops", &self.max_fee_drops)
            .finish_non_exhaustive()
    }
}

impl SchemeBuilder<XrplChainProvider> for XrplExact {
    fn build(
        &self,
        provider: XrplChainProvider,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        let parsed = config
            .map(serde_json::from_value::<XrplExactFacilitatorConfig>)
            .transpose()?
            .unwrap_or_default();
        let fee = parsed
            .max_fee_drops
            .unwrap_or_else(|| provider.max_fee_drops());
        Ok(Box::new(XrplExactFacilitator::new(provider, fee)))
    }
}

impl SchemeBuilder<&XrplChainProvider> for XrplExact {
    fn build(
        &self,
        provider: &XrplChainProvider,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        SchemeBuilder::<XrplChainProvider>::build(self, provider.clone(), config)
    }
}

impl Facilitator for XrplExactFacilitator<XrplChainProvider> {
    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_xrpl::exact::verify", skip_all)
    )]
    async fn verify(
        &self,
        request: wire::VerifyRequest,
    ) -> Result<wire::VerifyResponse, FacilitatorError> {
        let json = request.into_json();
        Ok(verify_request_json(&self.provider, self.max_fee_drops, &json).await)
    }

    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_xrpl::exact::settle", skip_all)
    )]
    async fn settle(
        &self,
        request: wire::SettleRequest,
    ) -> Result<wire::SettleResponse, FacilitatorError> {
        let json = request.into_json();
        let provider = self.provider.clone();
        Ok(settle_request(
            &self.provider,
            &self.settlement_cache,
            self.max_fee_drops,
            &json,
            move |blob, last_ledger| async move {
                let hash = crate::chain::codec::signed_tx_hash(&blob)
                    .map_err(|e| crate::chain::rpc::XrplRpcError::Parse(e.to_string()))?;
                provider
                    .rpc()
                    .submit_and_wait(&blob, &hash, last_ledger)
                    .await
            },
        )
        .await)
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<wire::SupportedResponse, FacilitatorError>> + Send {
        let chain_id = self.provider.chain_id();
        let extra = serde_json::json!({ "areFeesSponsored": false });
        let kinds = vec![
            wire::SupportedPaymentKind::new(
                wire::V2.into(),
                ExactScheme.to_string(),
                chain_id.to_string(),
            )
            .with_extra(extra),
        ];
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::with_capacity(1);
        let _ = signers.insert(XrplExact.caip_family().into(), Vec::new());
        std::future::ready(Ok(wire::SupportedResponse::new()
            .with_kinds(kinds)
            .with_signers(signers)))
    }
}
