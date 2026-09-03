//! Facilitator-side payment verification and settlement for the Keeta exact scheme.

mod queue;
mod settle;
mod verify;

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use compact_str::CompactString;
pub use queue::{QueueError, SettlementQueue};
use r402_facilitator::{Facilitator, SettlementCache};
use r402_protocol::error::FacilitatorError;
use r402_protocol::network::ChainProvider;
use r402_protocol::payment::{
    SettleRequest, SettleResponse, SupportedPaymentKind, SupportedResponse, V2, VerifyRequest,
    VerifyResponse,
};
use r402_protocol::scheme::SchemeId;
pub use settle::settle_request;
pub use verify::{decode_block, verify_request_json};

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

impl KeetaExactFacilitator<KeetaChainProvider> {
    /// Constructs a facilitator from a chain provider.
    ///
    /// Builds a per-fee-payer [`SettlementQueue`] on the provider's network.
    ///
    /// # Errors
    ///
    /// Currently infallible. [`Result`] so `try_new(provider)?` compiles.
    #[allow(
        clippy::unnecessary_wraps,
        reason = "Result so try_new(provider)? compiles"
    )]
    pub fn try_new(provider: KeetaChainProvider) -> Result<Self, FacilitatorError> {
        let accounts = provider
            .fee_payers()
            .iter()
            .map(|payer| Arc::clone(payer.account()))
            .collect();
        let queue = SettlementQueue::new(accounts, provider.chain_reference().client_network());
        Ok(Self::new(provider, queue))
    }
}

impl<P> std::fmt::Debug for KeetaExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeetaExactFacilitator")
            .field("queue", &self.queue)
            .finish_non_exhaustive()
    }
}

impl Facilitator for KeetaExactFacilitator<KeetaChainProvider> {
    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_keeta::exact::verify", skip_all)
    )]
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        let json = request.into_json();
        Ok(verify_request_json(&self.provider, &self.provider.fee_payer_ids(), &json).await)
    }

    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_keeta::exact::settle", skip_all)
    )]
    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
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
    ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
        let chain_id = self.provider.chain_id();
        let kinds = vec![SupportedPaymentKind::new(
            V2.into(),
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
        std::future::ready(Ok(SupportedResponse::new()
            .with_kinds(kinds)
            .with_signers(signers)))
    }
}
