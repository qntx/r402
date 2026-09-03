//! On-chain `claimWithSignature`.

use alloy_primitives::Bytes;
use alloy_provider::Provider;
use alloy_sol_types::SolCall;
use compact_str::CompactString;
use r402_protocol::error::FacilitatorError;
use r402_protocol::payment as wire;

use super::Eip155BatchSettlementFacilitator;
use super::contract::IBatchSettlement;
use super::encoding::to_sol_claims;
use super::response::facilitator_err_to_settle;
use super::send::{Broadcast, simulate_and_broadcast};
use crate::batch_settlement::errors::{
    AUTHORIZER_NOT_CONFIGURED, CLAIM_SIMULATION_FAILED, CLAIM_TRANSACTION_FAILED,
    INVALID_PAYLOAD_TYPE,
};
use crate::batch_settlement::payload::{BatchSettlementPayload, v2};
use crate::chain::Eip155MetaTransactionProvider;
use crate::error::Eip155ExactError;

pub(super) async fn execute_claim<P>(
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
    let BatchSettlementPayload::Claim {
        claims,
        claim_authorizer_signature,
    } = &payload.payload
    else {
        return Ok(super::response::settle_failure(
            INVALID_PAYLOAD_TYPE,
            None,
            "",
            network,
        ));
    };
    let Some(sig) = claim_authorizer_signature.clone() else {
        return Ok(super::response::settle_failure(
            AUTHORIZER_NOT_CONFIGURED,
            None,
            "",
            network,
        ));
    };
    let calldata = match claim_calldata(claims, sig) {
        Ok(c) => c,
        Err(e) => return facilitator_err_to_settle(e, None, network),
    };
    simulate_and_broadcast(
        &fac.provider,
        Broadcast {
            calldata,
            sim_reason: CLAIM_SIMULATION_FAILED,
            tx_reason: CLAIM_TRANSACTION_FAILED,
            pending_key: None,
            pending: fac.pending.as_ref(),
            payer: None,
            network,
            amount: None,
        },
    )
    .await
}

fn claim_calldata(
    claims: &[crate::batch_settlement::payload::VoucherClaim],
    sig: Bytes,
) -> Result<Bytes, FacilitatorError> {
    let sol_claims = to_sol_claims(claims)?;
    Ok(Bytes::from(
        IBatchSettlement::claimWithSignatureCall {
            voucherClaims: sol_claims,
            authorizerSignature: sig,
        }
        .abi_encode(),
    ))
}
