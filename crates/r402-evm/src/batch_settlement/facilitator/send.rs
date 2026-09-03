//! Simulate-then-broadcast helper shared by deposit / claim / settle / refund.

use alloy_network::TransactionBuilder;
use alloy_primitives::{Address, Bytes};
use alloy_provider::Provider;
use alloy_rpc_types_eth::TransactionRequest;
use compact_str::CompactString;
use r402_facilitator::PendingSettlementStore;
use r402_protocol::error::{ErrorReason, FacilitatorError};
use r402_protocol::payment::SettleResponse;

use super::response::{settle_failure, settle_success};
use crate::batch_settlement::payload::BATCH_SETTLEMENT_ADDRESS;
use crate::chain::{Eip155MetaTransactionProvider, MetaTransaction};
use crate::error::Eip155ExactError;

pub(super) struct Broadcast<'a> {
    pub calldata: Bytes,
    pub sim_reason: &'static str,
    pub tx_reason: &'static str,
    pub pending_key: Option<&'a str>,
    pub pending: &'a dyn PendingSettlementStore,
    pub payer: Option<Address>,
    pub network: CompactString,
    pub amount: Option<CompactString>,
}

pub(super) async fn simulate_and_broadcast<P>(
    provider: &P,
    spec: Broadcast<'_>,
) -> Result<SettleResponse, FacilitatorError>
where
    P: Eip155MetaTransactionProvider + Sync,
    P::Inner: Provider,
    Eip155ExactError: From<P::Error>,
{
    let req = TransactionRequest::default()
        .with_to(BATCH_SETTLEMENT_ADDRESS)
        .with_input(spec.calldata.clone());
    if provider.inner().call(req).await.is_err() {
        return Ok(settle_failure(
            spec.sim_reason,
            spec.payer,
            "",
            spec.network,
        ));
    }

    match Eip155MetaTransactionProvider::send_transaction(
        provider,
        MetaTransaction {
            to: BATCH_SETTLEMENT_ADDRESS,
            calldata: spec.calldata,
            confirmations: 1,
            from: None,
        },
    )
    .await
    {
        Ok(receipt) => {
            if let Some(key) = spec.pending_key {
                spec.pending.delete(key);
            }
            if receipt.status() {
                Ok(settle_success(
                    spec.payer,
                    receipt.transaction_hash,
                    spec.network,
                    spec.amount,
                ))
            } else {
                Ok(settle_failure(
                    spec.tx_reason,
                    spec.payer,
                    receipt.transaction_hash.to_string(),
                    spec.network,
                ))
            }
        }
        Err(e) => match Eip155ExactError::from(e) {
            Eip155ExactError::ReceiptWait { hash, .. } => {
                if let Some(key) = spec.pending_key {
                    spec.pending.set(key, CompactString::from(hash.to_string()));
                }
                Ok(settle_failure(
                    ErrorReason::SettlementPending.as_str(),
                    spec.payer,
                    hash.to_string(),
                    spec.network,
                ))
            }
            _ => Ok(settle_failure(spec.tx_reason, spec.payer, "", spec.network)),
        },
    }
}
