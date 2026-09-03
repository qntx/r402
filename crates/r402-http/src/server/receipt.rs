//! `Payment-Response` attach, failure-path receipts, skip-handler bodies.

use axum_core::body::Body;
use axum_core::response::Response;
use http::{HeaderValue, StatusCode, header::CONTENT_TYPE};
use r402_protocol::payment::SettleResponse;
use r402_server::{
    CompletedSettlement, SkipHandlerDirective, WirePaymentPayload,
    build_failure_path_settlement_response,
};

use super::fail::GateError;
use crate::headers::{PAYMENT_RESPONSE, ensure_expose_headers, merge_private};

/// Encodes a successful settlement as `Payment-Response`.
///
/// # Errors
///
/// [`GateError::SettlementAborted`] when the response is not success or encoding fails.
pub fn settlement_to_header(settlement: &SettleResponse) -> Result<HeaderValue, GateError> {
    let encoded = settlement
        .encode_base64()
        .ok_or_else(|| GateError::SettlementAborted("cannot encode error settlement".to_owned()))?;
    HeaderValue::from_bytes(encoded.as_ref())
        .map_err(|e| GateError::SettlementAborted(e.to_string()))
}

pub(crate) fn attach_payment_response(
    response: &mut Response,
    settlement: &SettleResponse,
) -> Result<(), GateError> {
    let header_value = settlement_to_header(settlement)?;
    let _ = response
        .headers_mut()
        .insert(PAYMENT_RESPONSE, header_value);
    ensure_expose_headers(response.headers_mut());
    merge_private(response.headers_mut());
    Ok(())
}

pub(crate) fn attach_failure_path_settlement(
    response: &mut Response,
    cancel_settlement: Option<&SettleResponse>,
    before_handler: Option<&CompletedSettlement>,
    payment_payload: Option<&WirePaymentPayload>,
) -> Result<(), GateError> {
    let Some(receipt) =
        build_failure_path_settlement_response(cancel_settlement, before_handler, payment_payload)
    else {
        return Ok(());
    };
    let encoded = receipt.encode_base64_any().ok_or_else(|| {
        GateError::SettlementAborted("cannot encode failure-path settlement".to_owned())
    })?;
    let header_value = HeaderValue::from_bytes(encoded.as_ref())
        .map_err(|e| GateError::SettlementAborted(e.to_string()))?;
    let _ = response
        .headers_mut()
        .insert(PAYMENT_RESPONSE, header_value);
    ensure_expose_headers(response.headers_mut());
    Ok(())
}

pub(crate) fn skip_handler_response(
    directive: &SkipHandlerDirective,
    settlement: &SettleResponse,
) -> Result<Response, GateError> {
    let content_type = directive
        .content_type
        .as_deref()
        .unwrap_or("application/json");
    let body_bytes = directive.body.as_ref().map_or_else(
        || b"null".to_vec(),
        |value| serde_json::to_vec(value).unwrap_or_else(|_| b"null".to_vec()),
    );
    let header_value = settlement_to_header(settlement)?;
    let mut response = Response::new(Body::from(body_bytes));
    *response.status_mut() = StatusCode::OK;
    if let Ok(ct) = HeaderValue::from_str(content_type) {
        let _ = response.headers_mut().insert(CONTENT_TYPE, ct);
    }
    let _ = response
        .headers_mut()
        .insert(PAYMENT_RESPONSE, header_value);
    ensure_expose_headers(response.headers_mut());
    merge_private(response.headers_mut());
    Ok(response)
}
