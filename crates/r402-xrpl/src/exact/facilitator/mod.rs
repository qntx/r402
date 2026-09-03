//! Facilitator-side payment verification and settlement for the XRPL exact scheme.

mod settle;
mod verify;

use std::collections::HashMap;
use std::future::Future;

use compact_str::CompactString;
use r402_facilitator::Facilitator;
use r402_protocol::error::FacilitatorError;
use r402_protocol::network::ChainProvider;
use r402_protocol::payment::{
    SettleRequest, SettleResponse, SupportedPaymentKind, SupportedResponse, V2, VerifyRequest,
    VerifyResponse,
};
use r402_protocol::scheme::SchemeId;
pub use settle::{XrplSettlementCache, settle_request, settlement_ttl};
pub use verify::{default_max_fee_drops, verify_request_json};

use crate::chain::XrplChainProvider;
use crate::chain::rpc::{XrplRpc, submit_and_wait};
use crate::exact::{ExactScheme, XrplExact};

/// Facilitator for XRPL exact scheme payments.
pub struct XrplExactFacilitator<P> {
    provider: P,
    max_fee_drops: u64,
    settlement_cache: XrplSettlementCache,
}

impl<P> XrplExactFacilitator<P> {
    /// Creates a facilitator with a per-entry landable-window settlement cache.
    #[must_use]
    pub fn new(provider: P, max_fee_drops: u64) -> Self {
        Self::with_settlement_cache(provider, max_fee_drops, XrplSettlementCache::new())
    }

    /// Creates a facilitator with a shared [`XrplSettlementCache`].
    #[must_use]
    pub const fn with_settlement_cache(
        provider: P,
        max_fee_drops: u64,
        settlement_cache: XrplSettlementCache,
    ) -> Self {
        Self {
            provider,
            max_fee_drops,
            settlement_cache,
        }
    }

    /// Overrides the maximum accepted transaction fee in drops.
    #[must_use]
    pub const fn with_max_fee_drops(mut self, drops: u64) -> Self {
        self.max_fee_drops = drops;
        self
    }
}

impl XrplExactFacilitator<XrplChainProvider> {
    /// Constructs a facilitator, copying [`XrplChainProvider::max_fee_drops`].
    ///
    /// # Errors
    ///
    /// Currently infallible. [`Result`] so `try_new(provider)?` compiles.
    #[allow(
        clippy::unnecessary_wraps,
        reason = "Result so try_new(provider)? compiles"
    )]
    pub fn try_new(provider: XrplChainProvider) -> Result<Self, FacilitatorError> {
        let max_fee_drops = provider.max_fee_drops();
        Ok(Self::new(provider, max_fee_drops))
    }
}

impl<P> std::fmt::Debug for XrplExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XrplExactFacilitator")
            .field("max_fee_drops", &self.max_fee_drops)
            .finish_non_exhaustive()
    }
}

impl<P> Facilitator for XrplExactFacilitator<P>
where
    P: XrplRpc + ChainProvider + Clone + 'static,
{
    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_xrpl::exact::verify", skip_all)
    )]
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        let json = request.into_json();
        Ok(verify_request_json(&self.provider, self.max_fee_drops, &json).await)
    }

    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_xrpl::exact::settle", skip_all)
    )]
    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        let json = request.into_json();
        let rpc = self.provider.clone();
        Ok(settle_request(
            &self.provider,
            &self.settlement_cache,
            self.max_fee_drops,
            &json,
            move |blob, last_ledger| async move {
                let hash = crate::chain::codec::signed_tx_hash(&blob)
                    .map_err(|e| crate::chain::rpc::XrplRpcError::Parse(e.to_string()))?;
                submit_and_wait(&rpc, &blob, &hash, last_ledger).await
            },
        )
        .await)
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
        let chain_id = self.provider.chain_id();
        let extra = serde_json::json!({ "areFeesSponsored": false });
        let kinds = vec![
            SupportedPaymentKind::new(V2.into(), ExactScheme.to_string(), chain_id.to_string())
                .with_extra(extra),
        ];
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::with_capacity(1);
        let _ = signers.insert(XrplExact.caip_family().into(), Vec::new());
        std::future::ready(Ok(SupportedResponse::new()
            .with_kinds(kinds)
            .with_signers(signers)))
    }
}
