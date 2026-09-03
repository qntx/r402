//! Parse the `SIGN-IN-WITH-X` header. Signature verification is not here.

use compact_str::CompactString;
use r402_protocol::payment::Base64Bytes;
use serde_json::Value;

use crate::siwx::SiwxError;

/// Failed to decode a `SIGN-IN-WITH-X` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SiwxProofError {
    /// Header value is not standard base64.
    #[error("SIGN-IN-WITH-X is not valid base64")]
    InvalidEncoding,
    /// Decoded bytes are not a JSON object.
    #[error("SIGN-IN-WITH-X is not a JSON object")]
    InvalidJson,
    /// A required proof field is missing or the wrong JSON type.
    #[error("SIGN-IN-WITH-X is missing a required field")]
    MissingField,
}

/// Client proof carried on `SIGN-IN-WITH-X` (base64 JSON).
///
/// Fields are stored; CAIP-122 verification is the SIWX PR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiwxProof {
    /// Wallet address that signed the message.
    pub address: CompactString,
    /// Cryptographic signature (hex for EVM, Base58 for Solana).
    pub signature: CompactString,
    /// CAIP-122 domain.
    pub domain: CompactString,
    /// Resource URI.
    pub uri: CompactString,
    /// Challenge nonce (32 hex).
    pub nonce: CompactString,
    /// ISO 8601 `issuedAt`.
    pub issued_at: CompactString,
    /// Optional ISO 8601 expiry.
    pub expiration_time: Option<CompactString>,
    /// CAIP-2 chain identifier.
    pub chain_id: CompactString,
}

impl SiwxProof {
    /// Decodes a `SIGN-IN-WITH-X` header value.
    ///
    /// # Errors
    ///
    /// [`SiwxProofError`] when the header is not base64 JSON with the
    /// required string fields.
    pub fn parse_header(value: &str) -> Result<Self, SiwxProofError> {
        let decoded = Base64Bytes(value.trim().as_bytes().to_vec())
            .decode()
            .map_err(|_| SiwxProofError::InvalidEncoding)?;
        let json: Value =
            serde_json::from_slice(&decoded).map_err(|_| SiwxProofError::InvalidJson)?;
        let obj = json.as_object().ok_or(SiwxProofError::InvalidJson)?;
        let field = |name: &str| -> Result<CompactString, SiwxProofError> {
            obj.get(name)
                .and_then(Value::as_str)
                .map(CompactString::from)
                .ok_or(SiwxProofError::MissingField)
        };
        Ok(Self {
            address: field("address")?,
            signature: field("signature")?,
            domain: field("domain")?,
            uri: field("uri")?,
            nonce: field("nonce")?,
            issued_at: field("issuedAt")?,
            expiration_time: obj
                .get("expirationTime")
                .and_then(Value::as_str)
                .map(CompactString::from),
            chain_id: field("chainId")?,
        })
    }

    /// Checks `domain` / `uri` against the configured origin. Never uses `Host`.
    ///
    /// # Errors
    ///
    /// [`SiwxError::DomainMismatch`] or [`SiwxError::UriMismatch`].
    pub fn bind_origin(&self, origin: &super::SiwxOrigin, path: &str) -> Result<(), SiwxError> {
        if self.domain != origin.domain() {
            return Err(SiwxError::DomainMismatch);
        }
        if self.uri != origin.uri(path) {
            return Err(SiwxError::UriMismatch);
        }
        Ok(())
    }
}
