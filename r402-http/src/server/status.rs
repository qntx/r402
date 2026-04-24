//! Maps x402 error reasons to HTTP status codes.
//!
//! Per x402 v2 §HTTP transport error mapping. The default status for a
//! payment failure is `402 Payment Required`; a few reasons use other
//! statuses to signal out-of-band preconditions:
//!
//! | [`ErrorReason`]                 | HTTP status              |
//! | ------------------------------- | ------------------------ |
//! | [`Permit2AllowanceRequired`]    | `412 Precondition Failed`|
//! | anything else                   | `402 Payment Required`   |
//!
//! [`Permit2AllowanceRequired`]: r402_core::error_reason::ErrorReason::Permit2AllowanceRequired

use http::StatusCode;
use r402_core::error_reason::ErrorReason;

/// Returns the HTTP status corresponding to an [`ErrorReason`].
#[must_use]
pub fn reason_to_status(reason: ErrorReason) -> StatusCode {
    match reason {
        ErrorReason::Permit2AllowanceRequired => StatusCode::PRECONDITION_FAILED,
        _ => StatusCode::PAYMENT_REQUIRED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permit2_allowance_required_maps_to_412() {
        assert_eq!(
            reason_to_status(ErrorReason::Permit2AllowanceRequired),
            StatusCode::PRECONDITION_FAILED,
        );
    }

    #[test]
    fn other_reasons_map_to_402() {
        for reason in [
            ErrorReason::InvalidFormat,
            ErrorReason::InvalidPaymentAmount,
            ErrorReason::InvalidSignature,
            ErrorReason::InsufficientFunds,
            ErrorReason::DuplicateSettlement,
            ErrorReason::MemoMismatch,
            ErrorReason::SimulationFailed,
            ErrorReason::UnexpectedError,
        ] {
            assert_eq!(
                reason_to_status(reason),
                StatusCode::PAYMENT_REQUIRED,
                "reason {reason:?} should map to 402"
            );
        }
    }
}
