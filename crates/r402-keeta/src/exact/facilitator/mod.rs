//! Facilitator-side payment verification and settlement for the Keeta exact scheme.

mod queue;
mod settle;
mod verify;

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use compact_str::CompactString;
pub use queue::{QueueError, SettlementQueue};
use r402_core::cache::SettlementCache;
use r402_core::chain::ChainProvider;
use r402_core::error::FacilitatorError;
use r402_core::facilitator::{DynFacilitator, Facilitator};
use r402_core::scheme::{SchemeBuilder, SchemeId};
use r402_core::wire;
pub use settle::settle_request;
pub use verify::{decode_block, verify_request_json};

#[cfg(test)]
mod tests;

use crate::chain::KeetaChainProvider;
use crate::exact::{ExactScheme, KeetaExact};

/// Facilitator for Keeta exact scheme payments.
pub struct KeetaExactFacilitator<P> {
    provider: P,
    queue: Arc<SettlementQueue>,
    settlement_cache: SettlementCache,
}

impl<P> KeetaExactFacilitator<P> {
    /// Creates a facilitator with a live per-account settlement queue.
    #[must_use]
    pub fn new(provider: P, queue: SettlementQueue) -> Self {
        Self::with_settlement_cache(provider, queue, SettlementCache::new())
    }

    /// Creates a facilitator with a shared [`SettlementCache`].
    #[must_use]
    pub fn with_settlement_cache(
        provider: P,
        queue: SettlementQueue,
        settlement_cache: SettlementCache,
    ) -> Self {
        Self {
            provider,
            queue: Arc::new(queue),
            settlement_cache,
        }
    }
}

impl<P> std::fmt::Debug for KeetaExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeetaExactFacilitator")
            .field("queue", &self.queue)
            .finish_non_exhaustive()
    }
}

impl SchemeBuilder<KeetaChainProvider> for KeetaExact {
    fn build(
        &self,
        provider: KeetaChainProvider,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        let _ = config;
        let accounts = provider
            .fee_payers()
            .iter()
            .map(|payer| Arc::clone(payer.account()))
            .collect();
        let queue = SettlementQueue::new(accounts, provider.chain_reference().client_network());
        Ok(Box::new(KeetaExactFacilitator::new(provider, queue)))
    }
}

impl SchemeBuilder<&KeetaChainProvider> for KeetaExact {
    fn build(
        &self,
        provider: &KeetaChainProvider,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        SchemeBuilder::<KeetaChainProvider>::build(self, provider.clone(), config)
    }
}

impl Facilitator for KeetaExactFacilitator<KeetaChainProvider> {
    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_keeta::exact::verify", skip_all)
    )]
    async fn verify(
        &self,
        request: wire::VerifyRequest,
    ) -> Result<wire::VerifyResponse, FacilitatorError> {
        let json = request.into_json();
        Ok(verify_request_json(&self.provider, &self.provider.fee_payer_ids(), &json).await)
    }

    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_keeta::exact::settle", skip_all)
    )]
    async fn settle(
        &self,
        request: wire::SettleRequest,
    ) -> Result<wire::SettleResponse, FacilitatorError> {
        let json = request.into_json();
        let fee_payer_ids = self.provider.fee_payer_ids();
        Ok(settle_request(
            &self.provider,
            &fee_payer_ids,
            &self.settlement_cache,
            &self.queue,
            &json,
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
            KeetaExact.caip_family().into(),
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
