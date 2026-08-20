//! Facilitator-side payment verification and settlement for the Stellar exact scheme.

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
pub use verify::{VerifiedStellar, verify_for_settle, verify_request_json};

#[cfg(test)]
mod tests;

use crate::chain::StellarChainProvider;
use crate::exact::{ExactScheme, StellarExact, StellarExtra};

/// Optional JSON config for [`StellarExact`] [`SchemeBuilder`].
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StellarExactFacilitatorConfig {
    /// Safety ceiling in stroops for simulation-derived settlement fees.
    #[serde(default)]
    pub max_transaction_fee_stroops: Option<u32>,
}

/// Facilitator for Stellar exact scheme payments.
pub struct StellarExactFacilitator<P> {
    provider: P,
    max_transaction_fee_stroops: u32,
    settlement_cache: SettlementCache,
}

impl StellarExactFacilitator<StellarChainProvider> {
    /// Creates a facilitator with the default in-memory settlement cache.
    #[must_use]
    pub fn new(provider: StellarChainProvider, max_transaction_fee_stroops: u32) -> Self {
        Self::with_settlement_cache(
            provider,
            max_transaction_fee_stroops,
            SettlementCache::new(),
        )
    }

    /// Creates a facilitator with a shared [`SettlementCache`].
    #[must_use]
    pub const fn with_settlement_cache(
        provider: StellarChainProvider,
        max_transaction_fee_stroops: u32,
        settlement_cache: SettlementCache,
    ) -> Self {
        Self {
            provider,
            max_transaction_fee_stroops,
            settlement_cache,
        }
    }
}

impl<P> std::fmt::Debug for StellarExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StellarExactFacilitator")
            .field(
                "max_transaction_fee_stroops",
                &self.max_transaction_fee_stroops,
            )
            .finish_non_exhaustive()
    }
}

impl SchemeBuilder<StellarChainProvider> for StellarExact {
    fn build(
        &self,
        provider: StellarChainProvider,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        let parsed = config
            .map(serde_json::from_value::<StellarExactFacilitatorConfig>)
            .transpose()?
            .unwrap_or_default();
        let fee = parsed
            .max_transaction_fee_stroops
            .unwrap_or_else(|| provider.max_transaction_fee_stroops());
        Ok(Box::new(StellarExactFacilitator::new(provider, fee)))
    }
}

impl SchemeBuilder<&StellarChainProvider> for StellarExact {
    fn build(
        &self,
        provider: &StellarChainProvider,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        SchemeBuilder::<StellarChainProvider>::build(self, provider.clone(), config)
    }
}

impl Facilitator for StellarExactFacilitator<StellarChainProvider> {
    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_stellar::exact::verify", skip_all)
    )]
    async fn verify(
        &self,
        request: wire::VerifyRequest,
    ) -> Result<wire::VerifyResponse, FacilitatorError> {
        let json = request.into_json();
        Ok(verify_request_json(
            &self.provider,
            &self.provider.signer_addresses(),
            self.max_transaction_fee_stroops,
            &json,
        )
        .await)
    }

    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_stellar::exact::settle", skip_all)
    )]
    async fn settle(
        &self,
        request: wire::SettleRequest,
    ) -> Result<wire::SettleResponse, FacilitatorError> {
        let json = request.into_json();
        let provider = self.provider.clone();
        let now = now_unix();
        Ok(settle_request(
            &self.provider,
            &self.provider.signer_addresses(),
            &self.settlement_cache,
            self.max_transaction_fee_stroops,
            &json,
            now,
            move |verified, max_timeout, now_unix| async move {
                provider
                    .settle_envelope(
                        &verified.envelope,
                        &verified.simulation,
                        max_timeout,
                        now_unix,
                    )
                    .await
            },
        )
        .await)
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<wire::SupportedResponse, FacilitatorError>> + Send {
        let chain_id = self.provider.chain_id();
        let extra = serde_json::to_value(StellarExtra::sponsored()).ok();
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
            StellarExact.caip_family().into(),
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
        .map_or(0, |d| d.as_secs())
}
