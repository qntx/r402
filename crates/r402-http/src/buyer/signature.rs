//! `Payment-Signature` encode and `Payment-Required` / `Payment-Response` decode.

use http::{HeaderMap, HeaderValue};
use r402_client::CreatedPayment;
use r402_protocol::ClientError;
use r402_protocol::payment::{Base64Bytes, PaymentRequired, SettleResponse};
use reqwest::{Request, Response};

use crate::headers::{PAYMENT_REQUIRED, PAYMENT_RESPONSE, PAYMENT_SIGNATURE};

/// Builds the `Payment-Signature` header map from a created payment.
///
/// # Errors
///
/// Returns [`ClientError::Parse`] when the signed payload is not a valid
/// HTTP header value.
pub fn payment_signature_headers(created: &CreatedPayment) -> Result<HeaderMap, ClientError> {
    let value = HeaderValue::from_str(&created.signed_payload).map_err(|err| {
        ClientError::Parse(format!(
            "signed payload is not a valid Payment-Signature header value: {err}"
        ))
    })?;
    let mut headers = HeaderMap::new();
    headers.insert(PAYMENT_SIGNATURE, value);
    Ok(headers)
}

/// Parses a 402 response into [`PaymentRequired`].
///
/// Tries the `Payment-Required` header (base64 JSON) first, then the body.
pub async fn parse_payment_required(response: Response) -> Option<PaymentRequired> {
    if let Some(required) = payment_required_from_headers(response.headers()) {
        return Some(required);
    }
    let body = response.bytes().await.ok()?;
    serde_json::from_slice(&body).ok()
}

pub(super) fn payment_required_from_headers(headers: &HeaderMap) -> Option<PaymentRequired> {
    serde_json::from_slice(&decode_b64_header(headers.get(PAYMENT_REQUIRED)?)?).ok()
}

pub(super) fn settle_response_from_headers(headers: &HeaderMap) -> Option<SettleResponse> {
    serde_json::from_slice(&decode_b64_header(headers.get(PAYMENT_RESPONSE)?)?).ok()
}

pub(super) fn clone_with_payment(
    template: &Request,
    created: &CreatedPayment,
) -> Result<Request, ClientError> {
    let mut req = template
        .try_clone()
        .ok_or(ClientError::RequestNotCloneable)?;
    req.headers_mut()
        .extend(payment_signature_headers(created)?);
    Ok(req)
}

fn decode_b64_header(value: &HeaderValue) -> Option<Vec<u8>> {
    Base64Bytes::from(value.as_bytes()).decode().ok()
}
