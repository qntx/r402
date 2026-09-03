//! On-chain settlement for a verified Solana exact payment.

use r402_facilitator::Duplicate;
use r402_protocol::payment::{Extensions, SettleRequest, SettleResponse};
use r402_protocol::{ChainProvider, FacilitatorError, VerificationError};
use solana_client::rpc_response::{TransactionError, UiTransactionError};
use solana_commitment_config::CommitmentConfig;
use solana_signature::Signature;
#[cfg(feature = "telemetry")]
use tracing_core::Level;

use super::verify::{VerifyTransferResult, verify_transfer};
use super::{SolanaExactFacilitator, enforce_memo};
use crate::chain::provider::{SolanaChainProviderError, SolanaChainProviderLike};
use crate::exact::payload::{self, TransactionInt};

/// Signs and submits a verified transaction.
///
/// # Errors
///
/// Returns [`SolanaChainProviderError`] if signing or confirmation fails.
pub async fn settle_transaction<P: SolanaChainProviderLike>(
    provider: &P,
    verification: VerifyTransferResult,
) -> Result<Signature, SolanaChainProviderError> {
    let tx = TransactionInt::new(verification.transaction).sign(provider)?;
    if !tx.is_fully_signed() {
        #[cfg(feature = "telemetry")]
        tracing::event!(Level::WARN, status = "failed", "undersigned transaction");
        return Err(SolanaChainProviderError::InvalidTransaction(
            UiTransactionError::from(TransactionError::SignatureFailure),
        ));
    }
    let tx_sig = tx
        .send_and_confirm(provider, CommitmentConfig::confirmed())
        .await?;
    Ok(tx_sig)
}

/// Deduplicates, re-verifies, and submits the payment.
pub(super) async fn settle<P>(
    facilitator: &SolanaExactFacilitator<P>,
    request: SettleRequest,
) -> Result<SettleResponse, FacilitatorError>
where
    P: SolanaChainProviderLike + ChainProvider + Send + Sync,
{
    let request = payload::v2::SettleRequest::from_settle(request)?;

    let cache_key = request.payment_payload.payload.transaction.as_str();
    if facilitator.settlement_cache.reserve(cache_key) == Duplicate::Yes {
        return Err(VerificationError::DuplicateSettlement.into());
    }

    enforce_memo(&request)?;
    let verification =
        verify_transfer(&facilitator.provider, &request, &facilitator.config).await?;
    let amount = verification.amount.to_string();
    let payer = verification.payer.to_string();
    let tx_sig = settle_transaction(&facilitator.provider, verification).await?;
    Ok(SettleResponse::Success {
        payer: payer.into(),
        transaction: tx_sig.to_string().into(),
        network: facilitator.provider.chain_id().to_string().into(),
        amount: Some(amount.into()),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    })
}
