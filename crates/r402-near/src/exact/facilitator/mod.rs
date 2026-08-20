//! Facilitator-side payment verification and settlement for the NEAR exact scheme.

mod settle;
mod verify;

use std::collections::HashMap;
use std::future::Future;

use compact_str::CompactString;
use r402_core::cache::SettlementCache;
use r402_core::chain::ChainProvider;
use r402_core::error::FacilitatorError;
use r402_core::facilitator::{DynFacilitator, Facilitator};
use r402_core::scheme::{SchemeBuilder, SchemeId};
use r402_core::wire;
use serde::Deserialize;
pub use settle::settle_request;
pub use verify::{decode_signed_delegate, default_max_sponsored_gas, verify_request_json};

#[cfg(test)]
mod tests;

use crate::chain::NearChainProvider;
use crate::exact::{ExactScheme, NearExact};

/// Optional JSON config for [`NearExact`] [`SchemeBuilder`].
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NearExactFacilitatorConfig {
    /// Maximum sponsored gas in gas units.
    ///
    /// When omitted, [`NearExact`] [`SchemeBuilder`] uses
    /// [`crate::chain::NearChainProvider::max_sponsored_gas`].
    #[serde(default)]
    pub max_sponsored_gas: Option<u64>,
}

/// Facilitator for NEAR exact scheme payments.
pub struct NearExactFacilitator<P> {
    provider: P,
    max_sponsored_gas: u64,
    settlement_cache: SettlementCache,
}

impl<P> NearExactFacilitator<P> {
    /// Creates a facilitator with the default in-memory settlement cache.
    pub fn new(provider: P, max_sponsored_gas: u64) -> Self {
        Self::with_settlement_cache(provider, max_sponsored_gas, SettlementCache::new())
    }

    /// Creates a facilitator with a shared [`SettlementCache`].
    pub const fn with_settlement_cache(
        provider: P,
        max_sponsored_gas: u64,
        settlement_cache: SettlementCache,
    ) -> Self {
        Self {
            provider,
            max_sponsored_gas,
            settlement_cache,
        }
    }
}

impl<P> std::fmt::Debug for NearExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NearExactFacilitator")
            .field("max_sponsored_gas", &self.max_sponsored_gas)
            .finish_non_exhaustive()
    }
}

impl SchemeBuilder<NearChainProvider> for NearExact {
    fn build(
        &self,
        provider: NearChainProvider,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        let parsed = config
            .map(serde_json::from_value::<NearExactFacilitatorConfig>)
            .transpose()?
            .unwrap_or_default();
        let gas = parsed
            .max_sponsored_gas
            .unwrap_or_else(|| provider.max_sponsored_gas());
        Ok(Box::new(NearExactFacilitator::new(provider, gas)))
    }
}

impl SchemeBuilder<&NearChainProvider> for NearExact {
    fn build(
        &self,
        provider: &NearChainProvider,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        SchemeBuilder::<NearChainProvider>::build(self, provider.clone(), config)
    }
}

impl Facilitator for NearExactFacilitator<NearChainProvider> {
    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_near::exact::verify", skip_all)
    )]
    async fn verify(
        &self,
        request: wire::VerifyRequest,
    ) -> Result<wire::VerifyResponse, FacilitatorError> {
        let json = request.into_json();
        Ok(verify_request_json(
            &self.provider,
            &self.provider.relayer_ids(),
            self.max_sponsored_gas,
            &json,
        )
        .await)
    }

    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_near::exact::settle", skip_all)
    )]
    async fn settle(
        &self,
        request: wire::SettleRequest,
    ) -> Result<wire::SettleResponse, FacilitatorError> {
        let json = request.into_json();
        let relayer_ids = self.provider.relayer_ids();
        let provider = self.provider.clone();
        Ok(settle_request(
            &self.provider,
            &relayer_ids,
            &self.settlement_cache,
            self.max_sponsored_gas,
            &json,
            move |relayer_id, signed_b64| async move {
                provider
                    .submit_signed_delegate(&relayer_id, &signed_b64)
                    .await
            },
        )
        .await)
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<wire::SupportedResponse, FacilitatorError>> + Send {
        let chain_id = self.provider.chain_id();
        let kinds = vec![wire::SupportedPaymentKind::new(
            wire::V2.into(),
            ExactScheme.to_string(),
            chain_id.to_string(),
        )];
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::with_capacity(1);
        let _ = signers.insert(
            NearExact.caip_family().into(),
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
