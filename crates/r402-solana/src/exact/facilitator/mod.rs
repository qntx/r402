//! Facilitator-side payment verification and settlement for Solana exact.

mod config;
mod memo;
mod settle;
mod smart_wallet;
mod verify;

use std::collections::HashMap;
use std::future::Future;

use compact_str::CompactString;
pub use config::SolanaExactFacilitatorConfig;
use r402_facilitator::{Facilitator, SettlementCache};
use r402_protocol::payment::{
    SettleRequest, SettleResponse, SupportedPaymentKind, SupportedResponse, VerifyRequest,
    VerifyResponse,
};
use r402_protocol::{ChainProvider, FacilitatorError, SchemeId, VerificationError};
pub use settle::settle_transaction;
pub use smart_wallet::{
    ObservedTransfer, extract_top_level_transfers, is_path1_layout_recoverable,
    match_required_transfer, parse_transfer_checked, smart_wallet_enabled,
};
pub use verify::{
    TransferCheckedInstruction, TransferRequirement, VerifyTransferResult,
    transfer_amount_meets_requirement, validate_instructions, verify_compute_limit_instruction,
    verify_compute_price_instruction, verify_transaction, verify_transfer,
    verify_transfer_instruction,
};

use crate::chain::provider::SolanaChainProviderLike;
use crate::exact::{ExactScheme, SolanaExact, SupportedPaymentKindExtra, payload};

/// Facilitator for Solana exact scheme payments.
pub struct SolanaExactFacilitator<P> {
    pub(super) provider: P,
    pub(super) config: SolanaExactFacilitatorConfig,
    pub(super) settlement_cache: SettlementCache,
}

impl<P> SolanaExactFacilitator<P> {
    /// Creates a facilitator with default settlement deduplication.
    #[must_use]
    pub fn new(provider: P, config: SolanaExactFacilitatorConfig) -> Self {
        Self::with_settlement_cache(provider, config, SettlementCache::new())
    }

    /// Builds a facilitator with default config.
    #[must_use]
    pub fn try_new(provider: P) -> Self {
        Self::new(provider, SolanaExactFacilitatorConfig::default())
    }

    /// Creates a facilitator with a shared [`SettlementCache`].
    #[must_use]
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

impl<P> Facilitator for SolanaExactFacilitator<P>
where
    P: SolanaChainProviderLike + ChainProvider + Send + Sync,
{
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        let request = payload::v2::VerifyRequest::from_verify(request)?;
        enforce_memo(&request)?;
        let verification = verify_transfer(&self.provider, &request, &self.config).await?;
        Ok(VerifyResponse::valid(verification.payer.to_string()))
    }

    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        settle::settle(self, request).await
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
        let chain_id = self.provider.chain_id();
        let kinds: Vec<SupportedPaymentKind> = {
            let fee_payer = self.provider.fee_payer();
            let extra = serde_json::to_value(SupportedPaymentKindExtra {
                fee_payer,
                memo: None,
            })
            .ok();
            vec![
                SupportedPaymentKind::new(
                    r402_protocol::V2.into(),
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
        std::future::ready(Ok(SupportedResponse::new()
            .with_kinds(kinds)
            .with_signers(signers)))
    }
}

/// Decodes the base64 transaction and, if `accepted.extra.memo` is declared,
/// enforces the memo instruction constraint.
pub(super) fn enforce_memo(request: &payload::v2::VerifyRequest) -> Result<(), FacilitatorError> {
    use r402_protocol::Base64Bytes;
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

    let raw = Base64Bytes::from(request.payment_payload.payload.transaction.as_bytes())
        .decode()
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
