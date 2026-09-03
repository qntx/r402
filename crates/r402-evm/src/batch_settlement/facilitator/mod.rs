//! Facilitator for batch-settlement: on-chain deposit/claim/settle/refund.
//!
//! Voucher `/verify` stays off-chain + [`ChannelStore`]. Voucher `/settle` is
//! Failure `invalid_batch_settlement_evm_payload_type` with `transaction: ""`.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use alloy_primitives::Address;
use alloy_provider::Provider;
use compact_str::CompactString;
use r402_facilitator::{Facilitator, InMemoryPendingSettlementStore, PendingSettlementStore};
use r402_protocol::error::{FacilitatorError, VerificationError};
use r402_protocol::network::ChainProvider;
use r402_protocol::payment::{
    SettleRequest, SettleResponse, SupportedPaymentKind, SupportedResponse, V2, VerifyRequest,
    VerifyResponse,
};
use r402_protocol::scheme::{BatchSettlementScheme, SchemeId};

use super::Eip155BatchSettlement;
use super::errors::INVALID_PAYLOAD_TYPE;
use super::payload::{BatchSettlementPayload, v2};
use super::store::{ChannelStore, MemoryChannelStore};
use super::verify::verify_offchain;
use crate::chain::Eip155MetaTransactionProvider;
use crate::error::Eip155ExactError;

mod claim;
mod contract;
mod deposit;
mod deposit_eip3009;
mod deposit_permit2;
mod encoding;
mod refund;
mod response;
mod send;
mod settle_call;
mod sigcheck;
mod validate;

/// Facilitator holding a shared channel store, 6492 allowlist, and pending store.
pub struct Eip155BatchSettlementFacilitator<P> {
    provider: P,
    store: Arc<dyn ChannelStore>,
    eip6492_allowed_factories: Vec<Address>,
    pending: Arc<dyn PendingSettlementStore>,
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
    /// Constructs a facilitator from a chain provider with in-memory stores.
    ///
    /// EIP-6492 factory allowlist defaults to empty (fail-closed).
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

    /// Builds with a custom channel store.
    #[must_use]
    pub fn with_store(provider: P, store: Arc<dyn ChannelStore>) -> Self {
        Self {
            provider,
            store,
            eip6492_allowed_factories: Vec::new(),
            pending: Arc::new(InMemoryPendingSettlementStore::new()),
        }
    }

    /// Allowlist of EIP-6492 factories for undeployed ERC-3009 deposits. Default empty.
    #[must_use]
    pub fn with_eip6492_allowed_factories(mut self, factories: Vec<Address>) -> Self {
        self.eip6492_allowed_factories = factories;
        self
    }

    /// Override the pending-settlement store (deposit retry reconcile).
    #[must_use]
    pub fn with_pending_store(mut self, store: Arc<dyn PendingSettlementStore>) -> Self {
        self.pending = store;
        self
    }

    /// Shared channel-store handle.
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

    fn supported_sync(&self) -> SupportedResponse {
        let chain_id = self.provider.chain_id();
        let kinds = vec![SupportedPaymentKind::new(
            V2.into(),
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
        SupportedResponse::new()
            .with_kinds(kinds)
            .with_signers(signers)
    }
}

impl<P> Facilitator for Eip155BatchSettlementFacilitator<P>
where
    P: Eip155MetaTransactionProvider + ChainProvider + Send + Sync,
    P::Inner: Provider,
    Eip155ExactError: From<P::Error>,
{
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        let typed = v2::VerifyRequest::from_verify(request)?;
        let chain_id = self.parse_chain_id()?;
        match &typed.payment_payload.payload {
            BatchSettlementPayload::Deposit { .. } => {
                deposit::verify_deposit(
                    self,
                    &typed.payment_payload,
                    &typed.payment_requirements,
                    chain_id,
                )
                .await
            }
            BatchSettlementPayload::Voucher { .. } => {
                let _charge = verify_offchain(
                    &typed.payment_payload,
                    &typed.payment_requirements,
                    chain_id,
                    self.store.as_ref(),
                )?;
                let payer = typed
                    .payment_payload
                    .payload
                    .channel_config()
                    .map(|c| c.payer)
                    .unwrap_or_default();
                Ok(VerifyResponse::valid(format!("{payer:#x}")))
            }
            BatchSettlementPayload::Refund { .. } => {
                refund::verify_refund(
                    self,
                    &typed.payment_payload,
                    &typed.payment_requirements,
                    chain_id,
                )
                .await
            }
            BatchSettlementPayload::Claim { .. } | BatchSettlementPayload::Settle { .. } => {
                Ok(VerifyResponse::invalid(
                    None,
                    r402_protocol::error::ErrorReason::from_wire(INVALID_PAYLOAD_TYPE),
                ))
            }
        }
    }

    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        let typed = v2::SettleRequest::from_settle(request)?;
        let network: CompactString = typed.payment_requirements.network.to_string().into();
        match &typed.payment_payload.payload {
            BatchSettlementPayload::Deposit { .. } => {
                deposit::settle_deposit(self, &typed.payment_payload, &typed.payment_requirements)
                    .await
            }
            BatchSettlementPayload::Claim { .. } => {
                claim::execute_claim(self, &typed.payment_payload, &typed.payment_requirements)
                    .await
            }
            BatchSettlementPayload::Settle { .. } => {
                settle_call::execute_settle(
                    self,
                    &typed.payment_payload,
                    &typed.payment_requirements,
                )
                .await
            }
            BatchSettlementPayload::Refund { .. }
                if typed.payment_payload.payload.is_enriched_refund() =>
            {
                refund::execute_refund(self, &typed.payment_payload, &typed.payment_requirements)
                    .await
            }
            BatchSettlementPayload::Voucher { .. } | BatchSettlementPayload::Refund { .. } => {
                Ok(response::settle_failure(
                    INVALID_PAYLOAD_TYPE,
                    typed
                        .payment_payload
                        .payload
                        .channel_config()
                        .map(|c| c.payer),
                    "",
                    network,
                ))
            }
        }
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
        std::future::ready(Ok(self.supported_sync()))
    }
}
