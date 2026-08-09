//! Facilitator for batch-settlement (voucher accounting + structure checks).
//!
//! On-chain deposit/claim/sweep are invoked by operators via separate claim
//! tooling; request path settle updates local [`ChannelStore`] charged totals
//! so subsequent vouchers remain monotonic.

use std::future::Future;
use std::sync::Arc;

use r402_core::chain::ChainProvider;
use r402_core::error::{FacilitatorError, VerificationError};
use r402_core::facilitator::{DynFacilitator, Facilitator};
use r402_core::scheme::SchemeBuilder;
use r402_core::wire::{self, Extensions, SettleResponse, SupportedPaymentKind, SupportedResponse};

use super::Eip155BatchSettlement;
use super::store::{ChannelStore, MemoryChannelStore};
use super::types::v2;
use super::verify::verify_offchain;
use crate::chain::Eip155MetaTransactionProvider;

/// Facilitator holding a shared channel store.
pub struct Eip155BatchSettlementFacilitator<P> {
    provider: P,
    store: Arc<dyn ChannelStore>,
}

impl<P> std::fmt::Debug for Eip155BatchSettlementFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Eip155BatchSettlementFacilitator")
            .finish_non_exhaustive()
    }
}

impl<P> Eip155BatchSettlementFacilitator<P>
where
    P: ChainProvider,
{
    /// Builds with an in-memory store.
    #[must_use]
    pub fn new(provider: P) -> Self {
        Self {
            provider,
            store: Arc::new(MemoryChannelStore::new()),
        }
    }

    /// Builds with a custom store.
    #[must_use]
    pub fn with_store(provider: P, store: Arc<dyn ChannelStore>) -> Self {
        Self { provider, store }
    }

    /// Shared store handle.
    #[must_use]
    pub fn store(&self) -> Arc<dyn ChannelStore> {
        Arc::clone(&self.store)
    }

    fn parse_chain_id(&self) -> Result<u64, FacilitatorError> {
        self.provider
            .chain_id()
            .reference()
            .parse::<u64>()
            .map_err(|e| {
                FacilitatorError::Verification(VerificationError::InvalidFormat(e.to_string()))
            })
    }

    fn verify_sync(
        &self,
        request: wire::VerifyRequest,
    ) -> Result<wire::VerifyResponse, FacilitatorError> {
        let typed = v2::VerifyRequest::from_verify(request)?;
        let chain_id = self.parse_chain_id()?;
        let _charge = verify_offchain(
            &typed.payment_payload,
            &typed.payment_requirements,
            chain_id,
            self.store.as_ref(),
        )?;
        let payer = typed.payment_payload.payload.channel_config().payer;
        Ok(wire::VerifyResponse::valid(format!("{payer:#x}")))
    }

    fn settle_sync(
        &self,
        request: wire::SettleRequest,
    ) -> Result<SettleResponse, FacilitatorError> {
        let typed = v2::SettleRequest::from_settle(request)?;
        let chain_id = self.parse_chain_id()?;
        let charge = verify_offchain(
            &typed.payment_payload,
            &typed.payment_requirements,
            chain_id,
            self.store.as_ref(),
        )?;
        let voucher = typed.payment_payload.payload.voucher();
        let channel_id = voucher.channel_id;
        let max = voucher.max_claimable_amount;

        if !matches!(
            typed.payment_payload.payload,
            super::types::BatchSettlementPayload::Refund { .. }
        ) && !self.store.try_charge(channel_id, charge, max)
        {
            return Err(FacilitatorError::Verification(
                VerificationError::InvalidFormat("charge exceeds voucher ceiling at settle".into()),
            ));
        }

        let payer = typed.payment_payload.payload.channel_config().payer;
        Ok(SettleResponse::Success {
            payer: format!("{payer:#x}").into(),
            // Off-chain accounting settle — no tx until claim batch is submitted.
            transaction: String::new().into(),
            network: typed.payment_requirements.network.to_string().into(),
            amount: Some(charge.0.to_string().into()),
            extensions: Extensions::new(),
        })
    }

    fn supported_sync(&self) -> SupportedResponse {
        let network = self.provider.chain_id().to_string();
        let kind = SupportedPaymentKind::new(2, "batch-settlement", network);
        SupportedResponse::new().with_kinds(vec![kind])
    }
}

impl<P> SchemeBuilder<P> for Eip155BatchSettlement
where
    P: Eip155MetaTransactionProvider + ChainProvider + Send + Sync + 'static,
{
    fn build(
        &self,
        provider: P,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        // Optional JSON: { "store": "memory" } only for now.
        let _ = config;
        Ok(Box::new(Eip155BatchSettlementFacilitator::new(provider)))
    }
}

impl<P> Facilitator for Eip155BatchSettlementFacilitator<P>
where
    P: Eip155MetaTransactionProvider + ChainProvider + Send + Sync,
{
    fn verify(
        &self,
        request: wire::VerifyRequest,
    ) -> impl Future<Output = Result<wire::VerifyResponse, FacilitatorError>> + Send {
        std::future::ready(self.verify_sync(request))
    }

    fn settle(
        &self,
        request: wire::SettleRequest,
    ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
        std::future::ready(self.settle_sync(request))
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
        std::future::ready(Ok(self.supported_sync()))
    }
}
