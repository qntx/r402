//! Facilitator-side payment verification and settlement for Solana exact scheme.
//!
//! This module implements the facilitator logic for verifying and settling SPL Token
//! payments on Solana.

mod config;
mod verify;

use std::collections::HashMap;

pub use config::SolanaExactFacilitatorConfig;
use r402::chain::ChainProvider;
use r402::facilitator::{BoxFuture, Facilitator, FacilitatorError};
use r402::proto;
use r402::proto::v2;
use r402::scheme::{SchemeBuilder, SchemeId};
pub use verify::{
    TransferCheckedInstruction, TransferRequirement, VerifyTransferResult, settle_transaction,
    validate_instructions, verify_compute_limit_instruction, verify_compute_price_instruction,
    verify_transaction, verify_transfer, verify_transfer_instruction,
};

use crate::chain::provider::SolanaChainProviderLike;
use crate::exact::types;
use crate::exact::{ExactScheme, SolanaExact, SupportedPaymentKindExtra};

impl<P> SchemeBuilder<P> for SolanaExact
where
    P: SolanaChainProviderLike + ChainProvider + Send + Sync + 'static,
{
    fn build(
        &self,
        provider: P,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn Facilitator>, Box<dyn std::error::Error>> {
        let config = config
            .map(serde_json::from_value::<SolanaExactFacilitatorConfig>)
            .transpose()?
            .unwrap_or_default();
        Ok(Box::new(SolanaExactFacilitator::new(provider, config)))
    }
}

/// Facilitator for Solana exact scheme payments.
pub struct SolanaExactFacilitator<P> {
    provider: P,
    config: SolanaExactFacilitatorConfig,
}

impl<P> std::fmt::Debug for SolanaExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SolanaExactFacilitator")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl<P> SolanaExactFacilitator<P> {
    /// Creates a new Solana exact facilitator.
    pub const fn new(provider: P, config: SolanaExactFacilitatorConfig) -> Self {
        Self { provider, config }
    }
}

impl<P> Facilitator for SolanaExactFacilitator<P>
where
    P: SolanaChainProviderLike + ChainProvider + Send + Sync,
{
    fn verify(
        &self,
        request: proto::VerifyRequest,
    ) -> BoxFuture<'_, Result<proto::VerifyResponse, FacilitatorError>> {
        Box::pin(async move {
            let request = types::v2::VerifyRequest::from_proto(request)?;
            let verification = verify_transfer(&self.provider, &request, &self.config).await?;
            Ok(v2::VerifyResponse::valid(verification.payer.to_string()))
        })
    }

    fn settle(
        &self,
        request: proto::SettleRequest,
    ) -> BoxFuture<'_, Result<proto::SettleResponse, FacilitatorError>> {
        Box::pin(async move {
            let request = types::v2::SettleRequest::from_settle(request)?;
            let verification = verify_transfer(&self.provider, &request, &self.config).await?;
            let payer = verification.payer.to_string();
            let tx_sig = settle_transaction(&self.provider, verification).await?;
            Ok(v2::SettleResponse::Success {
                payer,
                transaction: tx_sig.to_string(),
                network: self.provider.chain_id().to_string(),
                extensions: None,
            })
        })
    }

    fn supported(
        &self,
    ) -> BoxFuture<'_, Result<proto::SupportedResponse, FacilitatorError>> {
        Box::pin(async move {
            let chain_id = self.provider.chain_id();
            let kinds: Vec<proto::SupportedPaymentKind> = {
                let fee_payer = self.provider.fee_payer();
                let extra = serde_json::to_value(SupportedPaymentKindExtra { fee_payer }).ok();
                vec![proto::SupportedPaymentKind {
                    x402_version: v2::V2.into(),
                    scheme: ExactScheme.to_string(),
                    network: chain_id.to_string(),
                    extra,
                }]
            };
            let signers = {
                let mut signers = HashMap::with_capacity(1);
                signers.insert(SolanaExact.caip_family(), self.provider.signer_addresses());
                signers
            };
            Ok(proto::SupportedResponse {
                kinds,
                extensions: Vec::new(),
                signers,
            })
        })
    }
}
