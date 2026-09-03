//! Verify / settle response builders.

use alloy_primitives::{Address, TxHash};
use compact_str::CompactString;
use r402_protocol::error::{AsPaymentProblem, ErrorReason, FacilitatorError, VerificationError};
use r402_protocol::payment::{Extensions, SettleResponse};

pub(super) fn settle_success(
    payer: Option<Address>,
    hash: TxHash,
    network: CompactString,
    amount: Option<CompactString>,
) -> SettleResponse {
    SettleResponse::Success {
        payer: payer.map(|p| p.to_string().into()),
        transaction: hash.to_string().into(),
        network,
        amount,
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    }
}

pub(super) fn settle_failure(
    reason: impl AsRef<str>,
    payer: Option<Address>,
    transaction: impl Into<CompactString>,
    network: CompactString,
) -> SettleResponse {
    SettleResponse::Failure {
        reason: ErrorReason::from_wire(reason.as_ref()),
        message: None,
        payer: payer.map(|p| p.to_string().into()),
        transaction: transaction.into(),
        network,
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    }
}

pub(super) fn verify_err_to_settle(
    err: &VerificationError,
    payer: Option<Address>,
    network: CompactString,
) -> SettleResponse {
    let reason = err.as_payment_problem().reason().as_str().to_owned();
    settle_failure(reason, payer, "", network)
}

pub(super) fn facilitator_err_to_settle(
    err: FacilitatorError,
    payer: Option<Address>,
    network: CompactString,
) -> Result<SettleResponse, FacilitatorError> {
    match err {
        FacilitatorError::Verification(e) => Ok(verify_err_to_settle(&e, payer, network)),
        other => Err(other),
    }
}
