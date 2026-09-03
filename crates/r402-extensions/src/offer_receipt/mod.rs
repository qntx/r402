//! `offer-receipt` extension — signed offers on 402, signed receipts on 200.
//!
//! JWS compact (official subset: `ES256` / `ES256K` / `EdDSA`) and EIP-712
//! (`x402 offer` / `x402 receipt`, domain version `"1"`, `chainId` 1).

mod canonicalize;
mod client;
mod create;
mod did;
mod eip712;
mod jws;
mod server;

pub use canonicalize::{canonicalize, canonicalize_to_bytes};
pub use client::{
    DecodedOffer, decode_signed_offers, extract_offers_from_payment_required,
    extract_receipt_from_settle_response, find_accepts_object_from_signed_offer,
    verify_receipt_matches_offer,
};
pub use create::{
    DEFAULT_OFFER_VALIDITY_SECONDS, EXTENSION_VERSION, create_offer_eip712, create_offer_jws,
    create_receipt_eip712, create_receipt_jws, extract_offer_payload, extract_receipt_payload,
};
pub use did::public_key_from_kid;
pub use eip712::{
    Eip712DigestSigner, Eip712Verification, hash_offer_typed_data, hash_receipt_typed_data,
    verify_offer_signature_eip712, verify_receipt_signature_eip712,
};
pub use jws::{
    EddsaJwsSigner, Es256JwsSigner, Es256kJwsSigner, JwsHeader, JwsPublicKey, JwsSigner,
    create_jws, extract_jws_header, extract_jws_payload, verify_jws, verify_offer_signature_jws,
    verify_receipt_signature_jws,
};
use serde::{Deserialize, Serialize};
pub use server::{
    DynOfferReceiptIssuer, Eip712OfferReceiptIssuer, JwsOfferReceiptIssuer, OfferReceiptExtension,
    OfferReceiptIssuer, create_eip712_offer_receipt_issuer, create_jws_offer_receipt_issuer,
    declare_offer_receipt_extension,
};

/// Stable extension key on the wire.
pub const OFFER_RECEIPT: &str = "offer-receipt";

/// Offer/receipt signing format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignatureFormat {
    /// JWS compact serialization.
    Jws,
    /// EIP-712 typed data.
    Eip712,
}

/// Offer payload fields (spec §4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OfferPayload {
    /// Schema version (currently 1).
    pub version: u64,
    /// Paid resource URL.
    pub resource_url: String,
    /// Payment scheme identifier.
    pub scheme: String,
    /// CAIP-2 network.
    pub network: String,
    /// Token contract address or `"native"`.
    pub asset: String,
    /// Recipient wallet address.
    pub pay_to: String,
    /// Required payment amount.
    pub amount: String,
    /// Unix timestamp (seconds) when the offer expires.
    pub valid_until: u64,
}

/// Receipt payload fields (spec §5.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceiptPayload {
    /// Schema version (currently 1).
    pub version: u64,
    /// CAIP-2 network.
    pub network: String,
    /// Paid resource URL.
    pub resource_url: String,
    /// Payer identifier.
    pub payer: String,
    /// Unix timestamp (seconds) when the receipt was issued.
    pub issued_at: u64,
    /// Transaction hash; omitted in JWS when privacy-minimal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction: Option<String>,
}

/// Signed offer (JWS or EIP-712).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "format")]
pub enum SignedOffer {
    /// JWS compact; payload is inside `signature`.
    #[serde(rename = "jws")]
    Jws {
        /// Hint into `accepts[]`. Not signed.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "acceptIndex"
        )]
        accept_index: Option<u32>,
        /// `header.payload.signature`.
        signature: String,
    },
    /// EIP-712; `payload` is transmitted.
    #[serde(rename = "eip712")]
    Eip712 {
        /// Hint into `accepts[]`. Not signed.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "acceptIndex"
        )]
        accept_index: Option<u32>,
        /// Canonical payload.
        payload: OfferPayload,
        /// `0x`-prefixed 65-byte ECDSA signature.
        signature: String,
    },
}

impl SignedOffer {
    /// Unsigned `acceptIndex` hint.
    #[must_use]
    pub const fn accept_index(&self) -> Option<u32> {
        match self {
            Self::Jws { accept_index, .. } | Self::Eip712 { accept_index, .. } => *accept_index,
        }
    }
}

/// Signed receipt (JWS or EIP-712).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "format")]
pub enum SignedReceipt {
    /// JWS compact; payload is inside `signature`.
    #[serde(rename = "jws")]
    Jws {
        /// `header.payload.signature`.
        signature: String,
    },
    /// EIP-712; `payload` is transmitted.
    #[serde(rename = "eip712")]
    Eip712 {
        /// Canonical payload.
        payload: ReceiptPayload,
        /// `0x`-prefixed 65-byte ECDSA signature.
        signature: String,
    },
}

/// Inputs for creating an offer from a `PaymentRequirements` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferInput {
    /// Index into `accepts[]`.
    pub accept_index: u32,
    /// Payment scheme identifier.
    pub scheme: String,
    /// CAIP-2 network.
    pub network: String,
    /// Token contract address or `"native"`.
    pub asset: String,
    /// Recipient wallet address.
    pub pay_to: String,
    /// Required payment amount.
    pub amount: String,
    /// Offer validity duration in seconds. Default 300.
    pub offer_validity_seconds: Option<u64>,
}

/// Inputs for creating a receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptInput {
    /// Paid resource URL.
    pub resource_url: String,
    /// Payer identifier.
    pub payer: String,
    /// CAIP-2 network.
    pub network: String,
    /// Transaction hash when verifiability is requested.
    pub transaction: Option<String>,
}

/// Offer/receipt signing and verification failures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OfferReceiptError {
    /// JWS compact string is not three base64url parts.
    #[error("invalid JWS format")]
    InvalidJws,
    /// Canonical JSON serialization failed.
    #[error("cannot canonicalize: {0}")]
    Canonicalize(String),
    /// Signing backend failed.
    #[error("sign failed: {0}")]
    Sign(String),
    /// Signature verification failed.
    #[error("verify failed: {0}")]
    Verify(String),
    /// Unknown `format` field.
    #[error("unknown format: {0}")]
    UnknownFormat(String),
    /// System clock is before the Unix epoch.
    #[error("clock error")]
    Clock,
    /// No resource URL available to sign.
    #[error("missing resource URL")]
    MissingResourceUrl,
    /// DID method is not `key` or `jwk`.
    #[error("unsupported DID method: {0}")]
    UnsupportedDid(String),
    /// DID / JWK could not be parsed.
    #[error("invalid DID: {0}")]
    InvalidDid(String),
    /// JSON (de)serialization failed.
    #[error("{0}")]
    Serde(String),
}

impl From<serde_json::Error> for OfferReceiptError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serde(err.to_string())
    }
}
