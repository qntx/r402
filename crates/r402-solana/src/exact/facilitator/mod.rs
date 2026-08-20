//! Facilitator-side payment verification and settlement for Solana exact scheme.
//!
//! This module implements the facilitator logic for verifying and settling SPL Token
//! payments on Solana.

mod config;
mod memo;
mod smart_wallet;
mod verify;

use std::collections::HashMap;
use std::future::Future;

use compact_str::CompactString;
pub use config::SolanaExactFacilitatorConfig;
use r402_core::cache::{Duplicate, SettlementCache};
use r402_core::chain::ChainProvider;
use r402_core::error::FacilitatorError;
use r402_core::error::VerificationError;
use r402_core::facilitator::{DynFacilitator, Facilitator};
use r402_core::scheme::{SchemeBuilder, SchemeId};
use r402_core::wire;
pub use smart_wallet::{
    ObservedTransfer, extract_top_level_transfers, is_path1_layout_recoverable,
    match_required_transfer, parse_transfer_checked, smart_wallet_enabled,
};
pub use verify::{
    TransferCheckedInstruction, TransferRequirement, VerifyTransferResult, settle_transaction,
    transfer_amount_meets_requirement, validate_instructions, verify_compute_limit_instruction,
    verify_compute_price_instruction, verify_transaction, verify_transfer,
    verify_transfer_instruction,
};

use crate::chain::provider::SolanaChainProviderLike;
use crate::exact::types;
use crate::exact::{ExactScheme, SolanaExact, SupportedPaymentKindExtra};

/// Facilitator for Solana exact scheme payments.
pub struct SolanaExactFacilitator<P> {
    provider: P,
    config: SolanaExactFacilitatorConfig,
    settlement_cache: SettlementCache,
}

impl<P> SolanaExactFacilitator<P> {
    /// Creates a new Solana exact facilitator with the default in-memory
    /// settlement deduplication cache.
    pub fn new(provider: P, config: SolanaExactFacilitatorConfig) -> Self {
        Self::with_settlement_cache(provider, config, SettlementCache::new())
    }

    /// Creates a facilitator with a shared [`SettlementCache`].
    ///
    /// Sharing the cache across multiple facilitator instances
    /// guarantees a transaction submitted through one path cannot be replayed
    /// through another.
    pub const fn with_settlement_cache(
        provider: P,
        config: SolanaExactFacilitatorConfig,
        settlement_cache: SettlementCache,
    ) -> Self {
        Self {
            provider,
            config,
            settlement_cache,
        }
    }
}

impl<P> std::fmt::Debug for SolanaExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SolanaExactFacilitator")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl<P> SchemeBuilder<P> for SolanaExact
where
    P: SolanaChainProviderLike + ChainProvider + Send + Sync + 'static,
{
    fn build(
        &self,
        provider: P,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        let config = config
            .map(serde_json::from_value::<SolanaExactFacilitatorConfig>)
            .transpose()?
            .unwrap_or_default();
        Ok(Box::new(SolanaExactFacilitator::new(provider, config)))
    }
}

impl<P> Facilitator for SolanaExactFacilitator<P>
where
    P: SolanaChainProviderLike + ChainProvider + Send + Sync,
{
    async fn verify(
        &self,
        request: wire::VerifyRequest,
    ) -> Result<wire::VerifyResponse, FacilitatorError> {
        let request = types::v2::VerifyRequest::from_verify(request)?;
        enforce_memo(&request)?;
        let verification = verify_transfer(&self.provider, &request, &self.config).await?;
        Ok(wire::VerifyResponse::valid(verification.payer.to_string()))
    }

    async fn settle(
        &self,
        request: wire::SettleRequest,
    ) -> Result<wire::SettleResponse, FacilitatorError> {
        let request = types::v2::SettleRequest::from_settle(request)?;

        // Fix-2: Deduplicate by base64 transaction payload before running
        // on-chain settlement. Once reserved, a retried payload is rejected
        // with DuplicateSettlement until the cache TTL expires.
        let cache_key = request.payment_payload.payload.transaction.as_str();
        if self.settlement_cache.reserve(cache_key) == Duplicate::Yes {
            return Err(VerificationError::DuplicateSettlement.into());
        }

        enforce_memo(&request)?;
        let verification = verify_transfer(&self.provider, &request, &self.config).await?;
        // Report the on-chain TransferChecked amount (may exceed requirements
        // when the buyer overpays — see scheme_exact_svm.md §1.4).
        let amount = verification.amount.to_string();
        let payer = verification.payer.to_string();
        let tx_sig = settle_transaction(&self.provider, verification).await?;
        Ok(wire::SettleResponse::Success {
            payer: payer.into(),
            transaction: tx_sig.to_string().into(),
            network: self.provider.chain_id().to_string().into(),
            amount: Some(amount.into()),
            extensions: wire::Extensions::new(),
        })
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<wire::SupportedResponse, FacilitatorError>> + Send {
        let chain_id = self.provider.chain_id();
        let kinds: Vec<wire::SupportedPaymentKind> = {
            let fee_payer = self.provider.fee_payer();
            let extra = serde_json::to_value(SupportedPaymentKindExtra {
                fee_payer,
                memo: None,
            })
            .ok();
            vec![
                wire::SupportedPaymentKind::new(
                    wire::V2.into(),
                    ExactScheme.to_string(),
                    chain_id.to_string(),
                )
                .with_optional_extra(extra),
            ]
        };
        let signers = {
            let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::with_capacity(1);
            let _ = signers.insert(
                SolanaExact.caip_family().into(),
                self.provider
                    .signer_addresses()
                    .into_iter()
                    .map(CompactString::from)
                    .collect(),
            );
            signers
        };
        std::future::ready(Ok(wire::SupportedResponse::new()
            .with_kinds(kinds)
            .with_signers(signers)))
    }
}

/// Decodes the base64 transaction from `request.payment_payload.payload.transaction`
/// and, if `accepted.extra.memo` is declared, enforces the memo instruction
/// constraint.
fn enforce_memo(request: &types::v2::VerifyRequest) -> Result<(), FacilitatorError> {
    use base64::Engine;
    use solana_message::VersionedMessage;

    let Some(expected) = request
        .payment_payload
        .accepted
        .extra
        .as_ref()
        .and_then(|extra| extra.memo.as_deref())
    else {
        return Ok(());
    };

    let raw = base64::engine::general_purpose::STANDARD
        .decode(request.payment_payload.payload.transaction.as_bytes())
        .map_err(|e| VerificationError::InvalidFormat(e.to_string()))?;
    let transaction: solana_transaction::versioned::VersionedTransaction =
        bincode::deserialize(&raw).map_err(|e| VerificationError::InvalidFormat(e.to_string()))?;

    let (instructions, static_keys): (
        &[solana_message::compiled_instruction::CompiledInstruction],
        &[solana_pubkey::Pubkey],
    ) = match &transaction.message {
        VersionedMessage::Legacy(m) => (&m.instructions, &m.account_keys),
        VersionedMessage::V0(m) => (&m.instructions, &m.account_keys),
    };

    memo::verify_memo(instructions, static_keys, Some(expected))
        .map_err(|e| FacilitatorError::Verification(VerificationError::from(e)))?;
    Ok(())
}
