//! On-chain contract `settle(receiver, token)`.

use alloy_primitives::Bytes;
use alloy_provider::Provider;
use alloy_sol_types::SolCall;
use compact_str::CompactString;
use r402_protocol::error::FacilitatorError;
use r402_protocol::payment as wire;

use super::Eip155BatchSettlementFacilitator;
use super::contract::IBatchSettlement;
use super::response::settle_failure;
use super::send::{Broadcast, simulate_and_broadcast};
use crate::batch_settlement::errors::{
    INVALID_PAYLOAD_TYPE, NOTHING_TO_SETTLE, RPC_READ_FAILED, SETTLE_SIMULATION_FAILED,
    SETTLE_TRANSACTION_FAILED,
};
use crate::batch_settlement::payload::{BATCH_SETTLEMENT_ADDRESS, BatchSettlementPayload, v2};
use crate::chain::Eip155MetaTransactionProvider;
use crate::error::Eip155ExactError;

pub(super) async fn execute_settle<P>(
    fac: &Eip155BatchSettlementFacilitator<P>,
    payload: &v2::PaymentPayload,
    requirements: &v2::PaymentRequirements,
) -> Result<wire::SettleResponse, FacilitatorError>
where
    P: Eip155MetaTransactionProvider + Sync,
    P::Inner: Provider,
    Eip155ExactError: From<P::Error>,
{
    let network: CompactString = requirements.network.to_string().into();
    let BatchSettlementPayload::Settle { receiver, token } = &payload.payload else {
        return Ok(settle_failure(INVALID_PAYLOAD_TYPE, None, "", network));
    };
    let receiver = *receiver;
    let token = *token;
    let contract = IBatchSettlement::new(BATCH_SETTLEMENT_ADDRESS, fac.provider.inner());
    let Ok(state) = contract.receivers(receiver, token).call().await else {
        return Ok(settle_failure(RPC_READ_FAILED, None, "", network));
    };
    if state.totalClaimed <= state.totalSettled {
        return Ok(settle_failure(NOTHING_TO_SETTLE, None, "", network));
    }
    let calldata = Bytes::from(IBatchSettlement::settleCall { receiver, token }.abi_encode());
    simulate_and_broadcast(
        &fac.provider,
        Broadcast {
            calldata,
            sim_reason: SETTLE_SIMULATION_FAILED,
            tx_reason: SETTLE_TRANSACTION_FAILED,
            pending_key: None,
            pending: fac.pending.as_ref(),
            payer: None,
            network,
            amount: None,
        },
    )
    .await
}
