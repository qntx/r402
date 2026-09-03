//! Facilitator for batch-settlement (voucher accounting + structure checks).
//!
//! On-chain deposit/claim/sweep are invoked by operators via separate claim
//! tooling; request path settle updates local [`ChannelStore`] charged totals
//! so subsequent vouchers remain monotonic.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use compact_str::CompactString;
use r402_facilitator::Facilitator;
use r402_protocol::error::{FacilitatorError, VerificationError};
use r402_protocol::network::ChainProvider;
use r402_protocol::payment as wire;
use r402_protocol::scheme::{BatchSettlementScheme, SchemeId};

use super::Eip155BatchSettlement;
use super::payload::v2;
use super::store::{ChannelStore, MemoryChannelStore};
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
    /// Constructs a facilitator from a chain provider with an in-memory store.
    ///
    /// # Errors
    ///
    /// Currently infallible. [`Result`] so `try_new(provider)?` compiles.
    #[allow(
        clippy::unnecessary_wraps,
        reason = "Result so try_new(provider)? compiles"
    )]
    pub fn try_new(provider: P) -> Result<Self, FacilitatorError> {
        Ok(Self::with_store(
            provider,
            Arc::new(MemoryChannelStore::new()),
        ))
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
    ) -> Result<wire::SettleResponse, FacilitatorError> {
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

        if !self.store.try_charge(channel_id, charge, max) {
            return Err(FacilitatorError::Verification(
                VerificationError::InvalidFormat("charge exceeds voucher ceiling at settle".into()),
            ));
        }

        let payer = typed.payment_payload.payload.channel_config().payer;
        Ok(wire::SettleResponse::Success {
            payer: format!("{payer:#x}").into(),
            // Off-chain accounting settle — no tx until claim batch is submitted.
            transaction: String::new().into(),
            network: typed.payment_requirements.network.to_string().into(),
            amount: Some(charge.0.to_string().into()),
            extensions: wire::Extensions::new(),
            extension_responses: wire::Extensions::new(),
            extra: None,
        })
    }

    fn supported_sync(&self) -> wire::SupportedResponse {
        let chain_id = self.provider.chain_id();
        let kinds = vec![wire::SupportedPaymentKind::new(
            wire::V2.into(),
            BatchSettlementScheme.to_string(),
            chain_id.to_string(),
        )];
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::with_capacity(1);
        let _ = signers.insert(
            Eip155BatchSettlement.caip_family().into(),
            self.provider
                .signer_addresses()
                .into_iter()
                .map(CompactString::from)
                .collect(),
        );
        wire::SupportedResponse::new()
            .with_kinds(kinds)
            .with_signers(signers)
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
    ) -> impl Future<Output = Result<wire::SettleResponse, FacilitatorError>> + Send {
        std::future::ready(self.settle_sync(request))
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<wire::SupportedResponse, FacilitatorError>> + Send {
        std::future::ready(Ok(self.supported_sync()))
    }
}
