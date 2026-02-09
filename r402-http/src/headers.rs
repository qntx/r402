//! HTTP header encoding and decoding for x402 protocol messages.
//!
//! Handles Base64-encoded JSON payloads in `PAYMENT-SIGNATURE`,
//! `PAYMENT-REQUIRED`, `PAYMENT-RESPONSE`, and legacy `X-PAYMENT` headers.
//!
//! Corresponds to Python SDK's `http/x402_http_client_base.py`.

use base64::prelude::*;
use r402::proto::helpers::{PaymentPayloadEnum, PaymentRequiredEnum};
use r402::{
    PaymentPayload, PaymentPayloadV1, PaymentRequired, PaymentRequiredV1, SettleResponse,
};

use crate::error::HttpError;

/// Encodes a V2 [`PaymentPayload`] as a Base64 string for the
/// `PAYMENT-SIGNATURE` header.
///
/// # Errors
///
/// Returns [`HttpError::Serialize`] if JSON serialization fails.
pub fn encode_payment_signature(payload: &PaymentPayload) -> Result<String, HttpError> {
    let json = serde_json::to_vec(payload)?;
    Ok(BASE64_STANDARD.encode(&json))
}

/// Encodes a V1 [`PaymentPayloadV1`] as a Base64 string for the
/// `X-PAYMENT` header.
///
/// # Errors
///
/// Returns [`HttpError::Serialize`] if JSON serialization fails.
pub fn encode_x_payment(payload: &PaymentPayloadV1) -> Result<String, HttpError> {
    let json = serde_json::to_vec(payload)?;
    Ok(BASE64_STANDARD.encode(&json))
}

/// Decodes a `PAYMENT-SIGNATURE` or `X-PAYMENT` header value into a
/// version-tagged payload enum.
///
/// Attempts V2 first, then falls back to V1.
///
/// # Errors
///
/// Returns [`HttpError`] on Base64 or JSON decode failure.
pub fn decode_payment_payload(header_value: &str) -> Result<PaymentPayloadEnum, HttpError> {
    let bytes = BASE64_STANDARD.decode(header_value.trim())?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    r402::proto::helpers::parse_payment_payload(&value).map_err(HttpError::Protocol)
}

/// Encodes a [`PaymentRequired`] (V2) as a Base64 string for the
/// `PAYMENT-REQUIRED` header.
///
/// # Errors
///
/// Returns [`HttpError::Serialize`] if JSON serialization fails.
pub fn encode_payment_required(required: &PaymentRequired) -> Result<String, HttpError> {
    let json = serde_json::to_vec(required)?;
    Ok(BASE64_STANDARD.encode(&json))
}

/// Encodes a [`PaymentRequiredV1`] as a Base64 string for the
/// `PAYMENT-REQUIRED` header (V1).
///
/// # Errors
///
/// Returns [`HttpError::Serialize`] if JSON serialization fails.
pub fn encode_payment_required_v1(required: &PaymentRequiredV1) -> Result<String, HttpError> {
    let json = serde_json::to_vec(required)?;
    Ok(BASE64_STANDARD.encode(&json))
}

/// Decodes a `PAYMENT-REQUIRED` header value into a version-tagged enum.
///
/// # Errors
///
/// Returns [`HttpError`] on Base64 or JSON decode failure.
pub fn decode_payment_required(header_value: &str) -> Result<PaymentRequiredEnum, HttpError> {
    let bytes = BASE64_STANDARD.decode(header_value.trim())?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    r402::proto::helpers::parse_payment_required(&value).map_err(HttpError::Protocol)
}

/// Encodes a [`SettleResponse`] as a Base64 string for the
/// `PAYMENT-RESPONSE` header.
///
/// # Errors
///
/// Returns [`HttpError::Serialize`] if JSON serialization fails.
pub fn encode_payment_response(response: &SettleResponse) -> Result<String, HttpError> {
    let json = serde_json::to_vec(response)?;
    Ok(BASE64_STANDARD.encode(&json))
}

/// Decodes a `PAYMENT-RESPONSE` header value into a [`SettleResponse`].
///
/// # Errors
///
/// Returns [`HttpError`] on Base64 or JSON decode failure.
pub fn decode_payment_response(header_value: &str) -> Result<SettleResponse, HttpError> {
    let bytes = BASE64_STANDARD.decode(header_value.trim())?;
    Ok(serde_json::from_slice(&bytes)?)
}
