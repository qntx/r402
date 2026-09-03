//! Verify / settle response builders.

use alloy_primitives::{Address, TxHash};
use compact_str::CompactString;
use r402_protocol::error::{AsPaymentProblem, ErrorReason, FacilitatorError, VerificationError};
use r402_protocol::payment as wire;

pub(super) fn settle_success(
    payer: Option<Address>,
    hash: TxHash,
    network: CompactString,
    amount: Option<CompactString>,
) -> wire::SettleResponse {
    wire::SettleResponse::Success {
        payer: payer.map(|p| p.to_string().into()),
        transaction: hash.to_string().into(),
        network,
        amount,
        extensions: wire::Extensions::new(),
        extension_responses: wire::Extensions::new(),
        extra: None,
    }
}

pub(super) fn settle_failure(
    reason: impl AsRef<str>,
    payer: Option<Address>,
    transaction: impl Into<CompactString>,
    network: CompactString,
) -> wire::SettleResponse {
    wire::SettleResponse::Failure {
        reason: ErrorReason::from_wire(reason.as_ref()),
        message: None,
        payer: payer.map(|p| p.to_string().into()),
        transaction: transaction.into(),
        network,
        extensions: wire::Extensions::new(),
        extension_responses: wire::Extensions::new(),
        extra: None,
    }
}

pub(super) fn verify_err_to_settle(
    err: &VerificationError,
    payer: Option<Address>,
    network: CompactString,
) -> wire::SettleResponse {
    let reason = err.as_payment_problem().reason().as_str().to_owned();
    settle_failure(reason, payer, "", network)
}

pub(super) fn facilitator_err_to_settle(
    err: FacilitatorError,
    payer: Option<Address>,
    network: CompactString,
) -> Result<wire::SettleResponse, FacilitatorError> {
    match err {
        FacilitatorError::Verification(e) => Ok(verify_err_to_settle(&e, payer, network)),
        other => Err(other),
    }
}
