//! Gate errors and HTTP mapping (402 / 412 / 500 / 502).

use axum_core::body::Body;
use axum_core::response::Response;
use compact_str::CompactString;
use http::header::CONTENT_TYPE;
use http::{HeaderValue, StatusCode};
use r402_protocol::error::{ErrorReason, FacilitatorError, FacilitatorTransportKind};
use r402_protocol::network::ChainId;
use r402_protocol::payment::{Base64Bytes, SettleResponse};
use r402_server::{FacilitatorSupportError, PaymentFlowName};
use serde_json::json;

use super::gate::Gate;
use crate::headers::{PAYMENT_REQUIRED, PAYMENT_RESPONSE, ensure_expose_headers, set_no_store};

/// HTTP payment-gate failure.
#[derive(Debug, thiserror::Error)]
pub enum GateError {
    /// `Payment-Signature` is absent.
    #[error("Payment-Signature header is required")]
    PaymentHeaderMissing,
    /// `Payment-Signature` is present but not valid base64 JSON.
    #[error("Invalid or malformed payment header")]
    InvalidPaymentHeader,
    /// No accept matches the payload.
    #[error("Unable to find matching payment requirements")]
    NoPaymentMatching,
    /// Facilitator rejected the payment (`isValid: false` or verify error).
    #[error("Verification failed: {message}")]
    VerificationFailed {
        /// Wire reason used for 402 vs 412.
        reason: ErrorReason,
        /// Display text copied onto `PaymentRequired.error`.
        message: String,
    },
    /// Structured settle failure. Emitted as 402 + `Payment-Response`.
    #[error("settlement failed: {}", settlement_failure_summary(.0))]
    Settlement(Box<SettleResponse>),
    /// Settle never produced a structured body.
    #[error("settlement aborted: {0}")]
    SettlementAborted(String),
    /// 402 body construction failed.
    #[error("payment-required construction failed: {0}")]
    PaymentRequiredBuild(String),
    /// Concurrent/Background used with upfront or escrow.
    #[error("incompatible settlement mode {mode} with payment flow {flow}")]
    IncompatibleSettlementMode {
        /// Configured HTTP settlement mode.
        mode: super::SettlementMode,
        /// Offending payment flow.
        flow: PaymentFlowName,
    },
    /// No scheme registered for this accept.
    #[error("missing scheme {scheme} on {network}")]
    MissingScheme {
        /// Wire scheme name.
        scheme: CompactString,
        /// Accept network.
        network: ChainId,
    },
    /// Escrow accept whose scheme does not implement `settle_on_cancel`.
    #[error("escrow scheme {scheme} is missing settle_on_cancel")]
    MissingSettleOnCancel {
        /// Wire scheme name.
        scheme: CompactString,
    },
    /// Facilitator `/supported` kind missing or extra unusable. HTTP 500.
    #[error(transparent)]
    FacilitatorSupport(#[from] FacilitatorSupportError),
    /// Facilitator transport failure. HTTP 502.
    #[error("{kind}")]
    Transport {
        /// Transport failure kind.
        kind: FacilitatorTransportKind,
    },
}

const fn support_parts(err: &FacilitatorSupportError) -> (&CompactString, &ChainId) {
    match err {
        FacilitatorSupportError::KindMissing { scheme, network }
        | FacilitatorSupportError::MissingFeePayer { scheme, network }
        | FacilitatorSupportError::InvalidFeePayer { scheme, network }
        | FacilitatorSupportError::MissingReceiverAuthorizer { scheme, network }
        | FacilitatorSupportError::ZeroReceiverAuthorizer { scheme, network }
        | FacilitatorSupportError::InvalidReceiverAuthorizer { scheme, network } => {
            (scheme, network)
        }
    }
}

fn settlement_failure_summary(resp: &SettleResponse) -> String {
    match resp {
        SettleResponse::Failure {
            reason,
            message,
            network,
            ..
        } => format!(
            "{reason} ({network}){}",
            message
                .as_ref()
                .map(|m| format!(": {m}"))
                .unwrap_or_default(),
        ),
        SettleResponse::Success { .. } => "success returned via error path".to_owned(),
        _ => "unknown settlement variant".to_owned(),
    }
}

/// Maps an [`ErrorReason`] to 412 (Permit2) or 402.
#[must_use]
pub const fn reason_to_status(reason: &ErrorReason) -> StatusCode {
    match reason {
        ErrorReason::Permit2AllowanceRequired => StatusCode::PRECONDITION_FAILED,
        _ => StatusCode::PAYMENT_REQUIRED,
    }
}

impl GateError {
    pub(crate) fn from_verify_facilitator(err: FacilitatorError) -> Self {
        match err {
            FacilitatorError::Transport { kind } => Self::Transport { kind },
            other => {
                let reason = other
                    .as_payment_problem()
                    .map_or(ErrorReason::UnexpectedVerifyError, |problem| {
                        problem.reason()
                    });
                Self::VerificationFailed {
                    reason,
                    message: other.to_string(),
                }
            }
        }
    }

    pub(crate) fn from_settle_facilitator(err: FacilitatorError) -> Self {
        match err {
            FacilitatorError::Transport { kind } => Self::Transport { kind },
            other => Self::SettlementAborted(other.to_string()),
        }
    }

    pub(crate) fn from_invalid_verify(reason: Option<ErrorReason>, message: Option<&str>) -> Self {
        let reason = reason.unwrap_or(ErrorReason::UnexpectedVerifyError);
        Self::VerificationFailed {
            message: message.map_or_else(|| reason.to_string(), ToOwned::to_owned),
            reason,
        }
    }
}

impl Gate {
    /// Converts a [`GateError`] into an HTTP response.
    #[must_use]
    pub fn error_response(&self, err: GateError) -> Response {
        match err {
            GateError::PaymentHeaderMissing
            | GateError::InvalidPaymentHeader
            | GateError::NoPaymentMatching
            | GateError::VerificationFailed { .. } => challenge_response(self, &err),
            GateError::Settlement(failure) => settlement_failure_response(&failure),
            GateError::PaymentRequiredBuild(ref detail) => json_status_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &json!({ "error": detail }),
            ),
            GateError::IncompatibleSettlementMode { mode, flow } => json_status_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &json!({
                    "error": "incompatible settlement mode",
                    "mode": mode.as_str(),
                    "flow": flow.as_str(),
                }),
            ),
            GateError::MissingScheme {
                ref scheme,
                ref network,
            } => json_status_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &json!({
                    "error": "missing scheme",
                    "scheme": scheme,
                    "network": network.to_string(),
                }),
            ),
            GateError::MissingSettleOnCancel { ref scheme } => json_status_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &json!({
                    "error": "missing settle_on_cancel",
                    "scheme": scheme,
                }),
            ),
            GateError::FacilitatorSupport(ref err) => {
                let (scheme, network) = support_parts(err);
                json_status_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &json!({
                        "error": "facilitator support",
                        "scheme": scheme,
                        "network": network.to_string(),
                        "reason": err.reason(),
                    }),
                )
            }
            GateError::SettlementAborted(ref detail) => {
                let mut response = json_status_response(
                    StatusCode::PAYMENT_REQUIRED,
                    &json!({
                        "error": "settlement aborted",
                        "details": detail,
                    }),
                );
                set_no_store(response.headers_mut());
                response
            }
            GateError::Transport { kind } => json_status_response(
                StatusCode::BAD_GATEWAY,
                &json!({
                    "error": "facilitator transport",
                    "kind": kind.to_string(),
                }),
            ),
        }
    }
}

fn challenge_response(gate: &Gate, err: &GateError) -> Response {
    let Some(mut payment_required) = gate.payment_required().cloned() else {
        return json_status_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &json!({ "error": "payment-required response has not been built" }),
        );
    };
    let status = inferred_status(err);
    payment_required.error = Some(err.to_string().into());
    let Ok(body_bytes) = serde_json::to_vec(&payment_required) else {
        return json_status_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &json!({ "error": "payment-required serialization failed" }),
        );
    };
    let Ok(header_value) = HeaderValue::from_bytes(Base64Bytes::encode(&body_bytes).as_ref())
    else {
        return json_status_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &json!({ "error": "payment-required header encoding failed" }),
        );
    };
    let mut response = Response::new(Body::from(body_bytes));
    *response.status_mut() = status;
    let _ = response
        .headers_mut()
        .insert(PAYMENT_REQUIRED, header_value);
    let _ = response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    ensure_expose_headers(response.headers_mut());
    set_no_store(response.headers_mut());
    response
}

fn settlement_failure_response(failure: &SettleResponse) -> Response {
    let body_bytes = serde_json::to_vec(failure).unwrap_or_else(|_| b"{}".to_vec());
    let header_value = failure
        .encode_base64_any()
        .and_then(|b64| HeaderValue::from_bytes(b64.as_ref()).ok());
    let mut response = Response::new(Body::from(body_bytes));
    *response.status_mut() = StatusCode::PAYMENT_REQUIRED;
    let _ = response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(header_value) = header_value {
        let _ = response
            .headers_mut()
            .insert(PAYMENT_RESPONSE, header_value);
    }
    ensure_expose_headers(response.headers_mut());
    set_no_store(response.headers_mut());
    response
}

pub(crate) fn json_status_response(status: StatusCode, body: &serde_json::Value) -> Response {
    let mut response = Response::new(Body::from(body.to_string()));
    *response.status_mut() = status;
    let _ = response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    ensure_expose_headers(response.headers_mut());
    response
}

pub(crate) fn abort_response(status: StatusCode, body: Option<String>) -> Response {
    let mut response = Response::new(Body::from(body.unwrap_or_default()));
    *response.status_mut() = status;
    let _ = response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    ensure_expose_headers(response.headers_mut());
    response
}

const fn inferred_status(err: &GateError) -> StatusCode {
    if let GateError::VerificationFailed { reason, .. } = err {
        return reason_to_status(reason);
    }
    StatusCode::PAYMENT_REQUIRED
}
