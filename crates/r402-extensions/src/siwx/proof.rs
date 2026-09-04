//! Parse and CAIP-122-verify the `SIGN-IN-WITH-X` header.

use compact_str::CompactString;
use r402_protocol::payment::Base64Bytes;
use serde_json::Value;
use siwx::{SiwxMessage, Verifier};
use siwx_evm::EvmVerifier;
use siwx_svm::Ed25519Verifier;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::siwx::{DEFAULT_MAX_ISSUED_AGE, SiwxError, SiwxOrigin};

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
    /// `version` is not `"1"`.
    #[error("SIGN-IN-WITH-X has an invalid field")]
    InvalidField,
}

/// Client proof carried on `SIGN-IN-WITH-X` (base64 JSON).
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
    /// CAIP-122 version (always `"1"`).
    pub version: CompactString,
    /// Challenge nonce (32 hex).
    pub nonce: CompactString,
    /// ISO 8601 `issuedAt` as sent by the client (signing-string form).
    pub issued_at: CompactString,
    /// Optional ISO 8601 expiry, original string.
    pub expiration_time: Option<CompactString>,
    /// Optional ISO 8601 not-before, original string.
    pub not_before: Option<CompactString>,
    /// Optional CAIP-122 statement.
    pub statement: Option<CompactString>,
    /// Optional request id.
    pub request_id: Option<CompactString>,
    /// Optional resource URIs.
    pub resources: Vec<CompactString>,
    /// CAIP-2 chain identifier.
    pub chain_id: CompactString,
    /// Signature algorithm (`eip191` / `ed25519`).
    pub signature_type: CompactString,
    /// Spec `signatureScheme` hint, if the client sent one.
    pub signature_scheme: Option<CompactString>,
}

impl SiwxProof {
    /// Decodes a `SIGN-IN-WITH-X` header value.
    ///
    /// # Errors
    ///
    /// [`SiwxProofError`] when the header is not base64 JSON with the
    /// required string fields, or `version` is not `"1"`.
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
        let opt = |name: &str| -> Option<CompactString> {
            obj.get(name)
                .and_then(Value::as_str)
                .map(CompactString::from)
        };
        let resources = obj
            .get("resources")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(CompactString::from)
                    .collect()
            })
            .unwrap_or_default();
        let version = field("version")?;
        if version != "1" {
            return Err(SiwxProofError::InvalidField);
        }
        Ok(Self {
            address: field("address")?,
            signature: field("signature")?,
            domain: field("domain")?,
            uri: field("uri")?,
            version,
            nonce: field("nonce")?,
            issued_at: field("issuedAt")?,
            expiration_time: opt("expirationTime"),
            not_before: opt("notBefore"),
            statement: opt("statement"),
            request_id: opt("requestId"),
            resources,
            chain_id: field("chainId")?,
            signature_type: field("type")?,
            signature_scheme: opt("signatureScheme"),
        })
    }

    /// Checks `domain` / `uri` against the configured origin. Never uses `Host`.
    ///
    /// # Errors
    ///
    /// [`SiwxError::DomainMismatch`] or [`SiwxError::UriMismatch`].
    pub fn bind_origin(&self, origin: &SiwxOrigin, path: &str) -> Result<(), SiwxError> {
        if self.domain != origin.domain() {
            return Err(SiwxError::DomainMismatch);
        }
        if self.uri != origin.uri(path) {
            return Err(SiwxError::UriMismatch);
        }
        Ok(())
    }

    /// Origin, URI, and temporal checks. `now` is injectable for tests.
    ///
    /// # Errors
    ///
    /// Spec `invalid_siwx_*` field codes.
    pub fn validate_at(
        &self,
        origin: &SiwxOrigin,
        path: &str,
        now: OffsetDateTime,
    ) -> Result<(), SiwxError> {
        self.bind_origin(origin, path)?;
        let issued_at = parse_ts(&self.issued_at, SiwxError::IssuedAt)?;
        let age = now - issued_at;
        if age.is_negative() {
            return Err(SiwxError::IssuedAtInFuture);
        }
        if age > DEFAULT_MAX_ISSUED_AGE {
            return Err(SiwxError::IssuedAtTooOld);
        }
        if let Some(raw) = &self.expiration_time {
            let expiration = parse_ts(raw, SiwxError::ExpirationTime)?;
            if now > expiration {
                return Err(SiwxError::Expired);
            }
        }
        if let Some(raw) = &self.not_before {
            let not_before = parse_ts(raw, SiwxError::NotBefore)?;
            if now < not_before {
                return Err(SiwxError::NotYetValid);
            }
        }
        Ok(())
    }

    /// Canonical CAIP-122 signing string for this proof's fields.
    ///
    /// Renders via [`EvmVerifier::format_message`] /
    /// [`Ed25519Verifier::format_message`]. Timestamp fields are passed
    /// through `*_raw` so the original RFC 3339 bytes are hashed.
    ///
    /// # Errors
    ///
    /// [`SiwxError::ChainId`], [`SiwxError::UnsupportedChain`], or a field
    /// error when the proof cannot be turned into a [`SiwxMessage`].
    pub fn signing_message(&self) -> Result<String, SiwxError> {
        let (profile, message) = self.to_siwx_message()?;
        Ok(profile.format(&message))
    }

    /// Field validation plus CAIP-122 signature verification at `now`.
    ///
    /// # Errors
    ///
    /// Spec `invalid_siwx_*` codes.
    pub async fn verify_at(
        &self,
        origin: &SiwxOrigin,
        path: &str,
        now: OffsetDateTime,
        evm: &EvmVerifier,
    ) -> Result<(), SiwxError> {
        self.validate_at(origin, path, now)?;
        self.verify_signature(evm).await
    }

    /// [`Self::verify_at`] with the current UTC time and [`EvmVerifier::new`].
    ///
    /// # Errors
    ///
    /// Spec `invalid_siwx_*` codes.
    pub async fn verify(&self, origin: &SiwxOrigin, path: &str) -> Result<(), SiwxError> {
        self.verify_at(origin, path, OffsetDateTime::now_utc(), &EvmVerifier::new())
            .await
    }

    /// Encodes this proof as a `SIGN-IN-WITH-X` header value (base64 JSON).
    ///
    /// # Errors
    ///
    /// [`SiwxProofError::InvalidJson`] when the proof cannot be serialized.
    pub fn encode_header(&self) -> Result<String, SiwxProofError> {
        let mut body = serde_json::Map::new();
        let _ = body.insert("domain".into(), Value::String(self.domain.to_string()));
        let _ = body.insert("address".into(), Value::String(self.address.to_string()));
        let _ = body.insert("uri".into(), Value::String(self.uri.to_string()));
        let _ = body.insert("version".into(), Value::String(self.version.to_string()));
        let _ = body.insert("chainId".into(), Value::String(self.chain_id.to_string()));
        let _ = body.insert(
            "type".into(),
            Value::String(self.signature_type.to_string()),
        );
        if let Some(scheme) = &self.signature_scheme {
            let _ = body.insert("signatureScheme".into(), Value::String(scheme.to_string()));
        }
        let _ = body.insert("nonce".into(), Value::String(self.nonce.to_string()));
        let _ = body.insert("issuedAt".into(), Value::String(self.issued_at.to_string()));
        let _ = body.insert(
            "signature".into(),
            Value::String(self.signature.to_string()),
        );
        if let Some(statement) = &self.statement {
            let _ = body.insert("statement".into(), Value::String(statement.to_string()));
        }
        if let Some(expiration) = &self.expiration_time {
            let _ = body.insert(
                "expirationTime".into(),
                Value::String(expiration.to_string()),
            );
        }
        if let Some(not_before) = &self.not_before {
            let _ = body.insert("notBefore".into(), Value::String(not_before.to_string()));
        }
        if let Some(request_id) = &self.request_id {
            let _ = body.insert("requestId".into(), Value::String(request_id.to_string()));
        }
        if !self.resources.is_empty() {
            let resources = self
                .resources
                .iter()
                .map(|r| Value::String(r.to_string()))
                .collect();
            let _ = body.insert("resources".into(), Value::Array(resources));
        }
        let bytes =
            serde_json::to_vec(&Value::Object(body)).map_err(|_| SiwxProofError::InvalidJson)?;
        Ok(Base64Bytes::encode(bytes).to_string())
    }

    async fn verify_signature(&self, evm: &EvmVerifier) -> Result<(), SiwxError> {
        let (profile, message) = self.to_siwx_message()?;
        let raw = profile.format(&message);
        match profile {
            ChainProfile::Evm => {
                let sig = decode_evm_signature(&self.signature)?;
                map_crypto(evm.verify(&message, &raw, &sig).await, false)
            }
            ChainProfile::Svm => {
                let sig = decode_solana_signature(&self.signature)?;
                map_crypto(
                    Ed25519Verifier::new().verify(&message, &raw, &sig).await,
                    true,
                )
            }
        }
    }

    fn to_siwx_message(&self) -> Result<(ChainProfile, SiwxMessage), SiwxError> {
        if self.version != "1" {
            return Err(SiwxError::Signature);
        }
        let (profile, chain_ref) = chain_profile(&self.chain_id, &self.signature_type)?;
        match profile {
            ChainProfile::Evm => {
                EvmVerifier::validate_chain_id(chain_ref).map_err(|err| map_message_err(&err))?;
                if chain_ref == "0" {
                    return Err(SiwxError::ChainId);
                }
            }
            ChainProfile::Svm => {
                Ed25519Verifier::validate_chain_id(chain_ref)
                    .map_err(|err| map_message_err(&err))?;
            }
        }
        let mut message = SiwxMessage::new(
            self.domain.as_str(),
            self.address.as_str(),
            self.uri.as_str(),
            chain_ref,
            self.nonce.as_str(),
        )
        .map_err(|err| map_message_err(&err))?;
        match profile {
            ChainProfile::Evm => {
                EvmVerifier::validate_address(self.address.as_str())
                    .map_err(|err| map_message_err(&err))?;
            }
            ChainProfile::Svm => {
                Ed25519Verifier::validate_address(self.address.as_str())
                    .map_err(|err| map_message_err(&err))?;
            }
        }
        message = message
            .with_issued_at_raw(self.issued_at.as_str())
            .map_err(|_| SiwxError::IssuedAt)?;
        if let Some(statement) = &self.statement {
            message = message
                .with_statement(statement.as_str())
                .map_err(|_| SiwxError::Signature)?;
        }
        if let Some(exp) = &self.expiration_time {
            message = message
                .with_expiration_time_raw(exp.as_str())
                .map_err(|_| SiwxError::ExpirationTime)?;
        }
        if let Some(nbf) = &self.not_before {
            message = message
                .with_not_before_raw(nbf.as_str())
                .map_err(|_| SiwxError::NotBefore)?;
        }
        if let Some(rid) = &self.request_id {
            message = message
                .with_request_id(rid.as_str())
                .map_err(|_| SiwxError::Signature)?;
        }
        if !self.resources.is_empty() {
            message = message
                .with_resources(self.resources.iter().map(CompactString::as_str))
                .map_err(|_| SiwxError::Signature)?;
        }
        Ok((profile, message))
    }
}

fn parse_ts(raw: &str, on_err: SiwxError) -> Result<OffsetDateTime, SiwxError> {
    OffsetDateTime::parse(raw, &Rfc3339).map_err(|_| on_err)
}

#[derive(Clone, Copy, Debug)]
enum ChainProfile {
    Evm,
    Svm,
}

impl ChainProfile {
    fn format(self, message: &SiwxMessage) -> String {
        match self {
            Self::Evm => EvmVerifier::format_message(message),
            Self::Svm => Ed25519Verifier::format_message(message),
        }
    }
}

fn chain_profile<'a>(
    chain_id: &'a str,
    signature_type: &str,
) -> Result<(ChainProfile, &'a str), SiwxError> {
    if let Some(rest) = chain_id.strip_prefix("eip155:") {
        if signature_type != "eip191" {
            return Err(SiwxError::UnsupportedChain);
        }
        return Ok((ChainProfile::Evm, rest));
    }
    if let Some(rest) = chain_id.strip_prefix("solana:") {
        if signature_type != "ed25519" {
            return Err(SiwxError::UnsupportedChain);
        }
        return Ok((ChainProfile::Svm, rest));
    }
    Err(SiwxError::UnsupportedChain)
}

fn decode_evm_signature(signature: &str) -> Result<Vec<u8>, SiwxError> {
    let hex_str = signature
        .strip_prefix("0x")
        .or_else(|| signature.strip_prefix("0X"))
        .unwrap_or(signature);
    hex::decode(hex_str).map_err(|_| SiwxError::MalformedSignature)
}

fn decode_solana_signature(signature: &str) -> Result<Vec<u8>, SiwxError> {
    bs58::decode(signature)
        .into_vec()
        .map_err(|_| SiwxError::MalformedSignature)
}

fn map_crypto(result: Result<(), siwx::SiwxError>, solana: bool) -> Result<(), SiwxError> {
    result.map_err(|err| match err {
        siwx::SiwxError::InvalidSignature { .. } | siwx::SiwxError::InvalidAddress { .. } => {
            SiwxError::MalformedSignature
        }
        siwx::SiwxError::VerificationFailed { .. } => SiwxError::Signature,
        siwx::SiwxError::InvalidChainId { .. } => SiwxError::ChainId,
        _ if solana => SiwxError::MalformedSignature,
        _ => SiwxError::VerifierError,
    })
}

const fn map_message_err(err: &siwx::SiwxError) -> SiwxError {
    match err {
        siwx::SiwxError::InvalidNonce { .. } => SiwxError::Nonce,
        siwx::SiwxError::InvalidAddress { .. } => SiwxError::MalformedSignature,
        siwx::SiwxError::InvalidDomain { .. } => SiwxError::DomainMismatch,
        siwx::SiwxError::InvalidUri { .. } => SiwxError::UriMismatch,
        siwx::SiwxError::InvalidChainId { .. } => SiwxError::ChainId,
        _ => SiwxError::Signature,
    }
}
