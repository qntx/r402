//! Facilitator-side payment verification and settlement for Solana exact scheme.
//!
//! This module implements the facilitator logic for verifying and settling SPL Token
//! payments on Solana.

mod config;
mod verify;

pub use config::SolanaExactFacilitatorConfig;
pub use verify::{
    TransferCheckedInstruction, TransferRequirement, VerifyTransferResult, settle_transaction,
    validate_instructions, verify_compute_limit_instruction, verify_compute_price_instruction,
    verify_transaction, verify_transfer_instruction, verify_v2_transfer,
};

use r402::chain::ChainProvider;
use r402::facilitator::{Facilitator, FacilitatorError};
use r402::proto;
use r402::proto::v2;
use r402::scheme::{SchemeBuilder, SchemeId};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use crate::chain::provider::SolanaChainProviderLike;
use crate::exact::types;
use crate::exact::{ExactScheme, SupportedPaymentKindExtra, V2SolanaExact};

impl<P> SchemeBuilder<P> for V2SolanaExact
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
        Ok(Box::new(V2SolanaExactFacilitator::new(provider, config)))
    }
}

/// Facilitator for Solana exact scheme payments.
pub struct V2SolanaExactFacilitator<P> {
    provider: P,
    config: SolanaExactFacilitatorConfig,
}

impl<P> std::fmt::Debug for V2SolanaExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2SolanaExactFacilitator")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl<P> V2SolanaExactFacilitator<P> {
    /// Creates a new V2 Solana exact facilitator.
    pub const fn new(provider: P, config: SolanaExactFacilitatorConfig) -> Self {
        Self { provider, config }
    }
}

impl<P> Facilitator for V2SolanaExactFacilitator<P>
where
    P: SolanaChainProviderLike + ChainProvider + Send + Sync,
{
    fn verify(
        &self,
        request: proto::VerifyRequest,
    ) -> Pin<Box<dyn Future<Output = Result<proto::VerifyResponse, FacilitatorError>> + Send + '_>>
    {
        Box::pin(async move {
            let request = types::v2::VerifyRequest::from_proto(request)?;
            let verification = verify_v2_transfer(&self.provider, &request, &self.config).await?;
            Ok(v2::VerifyResponse::valid(verification.payer.to_string()))
        })
    }

    fn settle(
        &self,
        request: proto::SettleRequest,
    ) -> Pin<Box<dyn Future<Output = Result<proto::SettleResponse, FacilitatorError>> + Send + '_>>
    {
        Box::pin(async move {
            let request = types::v2::SettleRequest::from_settle(request)?;
            let verification = verify_v2_transfer(&self.provider, &request, &self.config).await?;
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
    ) -> Pin<Box<dyn Future<Output = Result<proto::SupportedResponse, FacilitatorError>> + Send + '_>>
    {
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
                signers.insert(
                    V2SolanaExact.caip_family(),
                    self.provider.signer_addresses(),
                );
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
