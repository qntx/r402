//! Protocol types for x402 payment messages.
//!
//! This module defines the wire format types used in the x402 protocol for
//! communication between buyers, sellers, and facilitators. It supports both
//! protocol version 1 (V1) and version 2 (V2).
//!
//! # Protocol Versions
//!
//! - **V1** ([`v1`]): Original protocol with network names and simpler structure
//! - **V2** ([`v2`]): Enhanced protocol with CAIP-2 chain IDs and richer metadata
//!
//! # Key Types
//!
//! - [`SupportedPaymentKind`] - Describes a payment method supported by a facilitator
//! - [`SupportedResponse`] - Response from facilitator's `/supported` endpoint
//! - [`VerifyRequest`] / [`VerifyResponse`] - Payment verification messages
//! - [`SettleRequest`] / [`SettleResponse`] - Payment settlement messages
//! - [`PaymentVerificationError`] - Errors that can occur during verification
//! - [`PaymentProblem`] - Structured error response for payment failures
//!
//! # Wire Format
//!
//! All types serialize to JSON using camelCase field names. The protocol version
//! is indicated by the `x402Version` field in payment payloads.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_with::{VecSkipError, serde_as};
use std::collections::HashMap;
use std::str::FromStr;

use crate::chain::ChainId;
use crate::scheme::SchemeSlug;

mod encoding;
mod error;
mod timestamp;
pub mod v1;
pub mod v2;
mod version;

pub use encoding::Base64Bytes;
pub use timestamp::UnixTimestamp;

pub use error::*;
pub use version::Version;

/// A version-tagged verify/settle request with typed payload and requirements.
///
/// This generic struct eliminates duplication between V1 and V2 verify requests.
/// The const parameter `V` selects the protocol version marker ([`Version<V>`]).
///
/// Use [`v1::VerifyRequest`] or [`v2::VerifyRequest`] type aliases instead of
/// constructing this directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypedVerifyRequest<const V: u8, TPayload, TRequirements> {
    /// The protocol version marker.
    pub x402_version: Version<V>,
    /// The signed payment authorization.
    pub payment_payload: TPayload,
    /// The payment requirements to verify against.
    pub payment_requirements: TRequirements,
}

impl<const V: u8, TPayload, TRequirements> TypedVerifyRequest<V, TPayload, TRequirements>
where
    Self: serde::de::DeserializeOwned,
{
    /// Deserializes from a protocol-level [`VerifyRequest`].
    ///
    /// # Errors
    ///
    /// Returns [`PaymentVerificationError`] if deserialization fails.
    pub fn from_proto(request: VerifyRequest) -> Result<Self, PaymentVerificationError> {
        let deserialized: Self = serde_json::from_value(request.into_json())?;
        Ok(deserialized)
    }

    /// Deserializes from a protocol-level [`SettleRequest`].
    ///
    /// Settlement reuses the same wire format as verification.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentVerificationError`] if deserialization fails.
    pub fn from_settle(request: SettleRequest) -> Result<Self, PaymentVerificationError> {
        let deserialized: Self = serde_json::from_value(request.into_json())?;
        Ok(deserialized)
    }
}

impl<const V: u8, TPayload, TRequirements> TryInto<VerifyRequest>
    for TypedVerifyRequest<V, TPayload, TRequirements>
where
    TPayload: Serialize,
    TRequirements: Serialize,
{
    type Error = serde_json::Error;
    fn try_into(self) -> Result<VerifyRequest, Self::Error> {
        let json = serde_json::to_value(self)?;
        Ok(VerifyRequest(json))
    }
}

/// Protocol extension data attached to various x402 wire types.
///
/// Keys are extension names; values are arbitrary JSON data specific to each extension.
pub type Extensions = HashMap<String, serde_json::Value>;

/// A `u64` value that serializes as a string.
///
/// Some JSON parsers (particularly in `JavaScript`) cannot accurately represent
/// large integers. This type serializes `u64` values as strings to preserve
/// precision across all platforms.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct U64String(u64);

impl U64String {
    /// Returns the inner `u64` value.
    #[must_use]
    pub const fn inner(&self) -> u64 {
        self.0
    }
}

impl FromStr for U64String {
    type Err = <u64 as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u64>().map(Self)
    }
}

impl From<u64> for U64String {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<U64String> for u64 {
    fn from(value: U64String) -> Self {
        value.0
    }
}

impl Serialize for U64String {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for U64String {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse::<u64>().map(Self).map_err(serde::de::Error::custom)
    }
}

/// Describes a payment method supported by a facilitator.
///
/// This type is returned in the [`SupportedResponse`] to indicate what
/// payment schemes, networks, and protocol versions a facilitator can handle.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportedPaymentKind {
    /// The x402 protocol version (1 or 2).
    pub x402_version: u8,
    /// The payment scheme identifier (e.g., "exact").
    pub scheme: String,
    /// The network identifier (CAIP-2 chain ID for V2, network name for V1).
    pub network: String,
    /// Optional scheme-specific extra data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

/// Response from a facilitator's `/supported` endpoint.
///
/// This response tells clients what payment methods the facilitator supports,
/// including protocol versions, schemes, networks, and signer addresses.
#[serde_as]
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportedResponse {
    /// List of supported payment kinds.
    #[serde_as(as = "VecSkipError<_>")]
    pub kinds: Vec<SupportedPaymentKind>,
    /// List of supported protocol extensions.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Map of CAIP-2 patterns to signer addresses.
    ///
    /// Keys can be exact chain IDs (e.g., `"eip155:8453"`) or wildcard patterns
    /// (e.g., `"eip155:*"`), matching the official x402 wire format.
    #[serde(default)]
    pub signers: HashMap<String, Vec<String>>,
}

impl SupportedResponse {
    /// Finds signer addresses that match the given chain ID.
    ///
    /// Checks both exact match (e.g., `"eip155:8453"`) and namespace wildcard
    /// (e.g., `"eip155:*"`).
    #[must_use]
    pub fn signers_for_chain(&self, chain_id: &ChainId) -> Vec<&str> {
        let exact_key = chain_id.to_string();
        let wildcard_key = format!("{}:*", chain_id.namespace());

        let mut result = Vec::new();
        if let Some(addrs) = self.signers.get(&exact_key) {
            result.extend(addrs.iter().map(String::as_str));
        }
        if let Some(addrs) = self.signers.get(&wildcard_key) {
            result.extend(addrs.iter().map(String::as_str));
        }
        result
    }
}

/// Request to verify a payment before settlement.
///
/// This wrapper contains the payment payload and requirements sent by a client
/// to a facilitator for verification. The facilitator checks that the payment
/// authorization is valid, properly signed, and matches the requirements.
///
/// The inner JSON structure varies by protocol version and scheme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyRequest(serde_json::Value);

/// Request to settle a verified payment on-chain.
///
/// Structurally identical to [`VerifyRequest`] on the wire, but represented as a
/// distinct type so the compiler can prevent accidental misuse (e.g., passing a
/// verify request where a settle request is expected).
///
/// Use `From<VerifyRequest>` to convert a verified request into a settle request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettleRequest(serde_json::Value);

impl SettleRequest {
    /// Consumes the request and returns the inner JSON value.
    #[must_use]
    pub fn into_json(self) -> serde_json::Value {
        self.0
    }

    /// Extracts the scheme handler slug from the request.
    ///
    /// Delegates to the same logic as [`VerifyRequest::scheme_slug`].
    #[must_use]
    pub fn scheme_slug(
        &self,
        registry: &crate::networks::NetworkRegistry,
    ) -> Option<SchemeSlug> {
        // Reuse VerifyRequest's implementation via a temporary reference-based parse.
        let tmp = VerifyRequest(self.0.clone());
        tmp.scheme_slug(registry)
    }
}

impl From<serde_json::Value> for SettleRequest {
    fn from(value: serde_json::Value) -> Self {
        Self(value)
    }
}

impl From<VerifyRequest> for SettleRequest {
    fn from(request: VerifyRequest) -> Self {
        Self(request.into_json())
    }
}

impl From<serde_json::Value> for VerifyRequest {
    fn from(value: serde_json::Value) -> Self {
        Self(value)
    }
}

impl VerifyRequest {
    /// Consumes the request and returns the inner JSON value.
    #[must_use]
    pub fn into_json(self) -> serde_json::Value {
        self.0
    }

    /// Extracts the scheme handler slug from the request.
    ///
    /// This determines which scheme handler should process this payment
    /// based on the protocol version, chain ID, and scheme name.
    ///
    /// For V1 requests, a [`NetworkRegistry`](crate::networks::NetworkRegistry) is
    /// required to resolve human-readable network names to CAIP-2 chain IDs.
    ///
    /// Returns `None` if the request format is invalid or the scheme is unknown.
    #[must_use]
    pub fn scheme_slug(
        &self,
        registry: &crate::networks::NetworkRegistry,
    ) -> Option<SchemeSlug> {
        let x402_version: u8 = self.0.get("x402Version")?.as_u64()?.try_into().ok()?;
        match x402_version {
            v1::X402Version1::VALUE => {
                let network_name = self.0.get("paymentPayload")?.get("network")?.as_str()?;
                let chain_id = registry.chain_id_by_name(network_name)?;
                let scheme = self.0.get("paymentPayload")?.get("scheme")?.as_str()?;
                let slug = SchemeSlug::new(chain_id.clone(), 1, scheme.into());
                Some(slug)
            }
            v2::X402Version2::VALUE => {
                let chain_id_string = self
                    .0
                    .get("paymentPayload")?
                    .get("accepted")?
                    .get("network")?
                    .as_str()?;
                let chain_id = ChainId::from_str(chain_id_string).ok()?;
                let scheme = self
                    .0
                    .get("paymentPayload")?
                    .get("accepted")?
                    .get("scheme")?
                    .as_str()?;
                let slug = SchemeSlug::new(chain_id, 2, scheme.into());
                Some(slug)
            }
            _ => None,
        }
    }
}

/// Result returned by a facilitator after verifying a payment payload
/// against the provided payment requirements.
///
/// This response indicates whether the payment authorization is valid and identifies
/// the payer. If invalid, it includes a reason describing why verification failed
/// (e.g., wrong network, invalid scheme, insufficient funds).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(into = "VerifyResponseWire", try_from = "VerifyResponseWire")]
#[non_exhaustive]
pub enum VerifyResponse {
    /// The payload matches the requirements and passes all checks.
    Valid {
        /// The address of the payer.
        payer: String,
    },
    /// The payload was well-formed but failed verification.
    Invalid {
        /// Machine-readable reason verification failed.
        reason: String,
        /// Optional human-readable description of the failure.
        message: Option<String>,
        /// The payer address, if identifiable.
        payer: Option<String>,
    },
}

impl VerifyResponse {
    /// Constructs a successful verification response with the given payer address.
    #[must_use]
    pub const fn valid(payer: String) -> Self {
        Self::Valid { payer }
    }

    /// Constructs a failed verification response.
    #[must_use]
    pub const fn invalid(payer: Option<String>, reason: String) -> Self {
        Self::Invalid {
            reason,
            message: None,
            payer,
        }
    }

    /// Constructs a failed verification response with a human-readable message.
    #[must_use]
    pub const fn invalid_with_message(
        payer: Option<String>,
        reason: String,
        message: String,
    ) -> Self {
        Self::Invalid {
            reason,
            message: Some(message),
            payer,
        }
    }

    /// Returns `true` if the verification succeeded.
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        matches!(self, Self::Valid { .. })
    }
}

/// Wire format for [`VerifyResponse`], using a flat boolean discriminator.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerifyResponseWire {
    is_valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    payer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invalid_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invalid_message: Option<String>,
}

impl From<VerifyResponse> for VerifyResponseWire {
    fn from(resp: VerifyResponse) -> Self {
        match resp {
            VerifyResponse::Valid { payer } => Self {
                is_valid: true,
                payer: Some(payer),
                invalid_reason: None,
                invalid_message: None,
            },
            VerifyResponse::Invalid {
                reason,
                message,
                payer,
            } => Self {
                is_valid: false,
                payer,
                invalid_reason: Some(reason),
                invalid_message: message,
            },
        }
    }
}

impl TryFrom<VerifyResponseWire> for VerifyResponse {
    type Error = String;

    fn try_from(wire: VerifyResponseWire) -> Result<Self, Self::Error> {
        if wire.is_valid {
            let payer = wire.payer.ok_or("missing field: payer")?;
            Ok(Self::Valid { payer })
        } else {
            let reason = wire.invalid_reason.ok_or("missing field: invalidReason")?;
            Ok(Self::Invalid {
                reason,
                message: wire.invalid_message,
                payer: wire.payer,
            })
        }
    }
}

/// Response from a payment settlement request.
///
/// Indicates whether the payment was successfully settled on-chain,
/// including the transaction hash and payer address on success.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(into = "SettleResponseWire", try_from = "SettleResponseWire")]
#[non_exhaustive]
pub enum SettleResponse {
    /// Settlement succeeded.
    Success {
        /// The address that paid.
        payer: String,
        /// The on-chain transaction hash.
        transaction: String,
        /// The network where settlement occurred (CAIP-2 chain ID or network name).
        network: String,
        /// Optional protocol extensions returned by the facilitator.
        extensions: Option<Extensions>,
    },
    /// Settlement failed.
    Error {
        /// Machine-readable reason for failure.
        reason: String,
        /// Optional human-readable description of the failure.
        message: Option<String>,
        /// The network where settlement was attempted.
        network: String,
    },
}

impl SettleResponse {
    /// Returns `true` if the settlement succeeded.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }
}

/// Wire format for [`SettleResponse`], using a flat boolean discriminator.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettleResponseWire {
    success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transaction: Option<String>,
    network: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extensions: Option<Extensions>,
}

impl From<SettleResponse> for SettleResponseWire {
    fn from(resp: SettleResponse) -> Self {
        match resp {
            SettleResponse::Success {
                payer,
                transaction,
                network,
                extensions,
            } => Self {
                success: true,
                error_reason: None,
                error_message: None,
                payer: Some(payer),
                transaction: Some(transaction),
                network,
                extensions,
            },
            SettleResponse::Error {
                reason,
                message,
                network,
            } => Self {
                success: false,
                error_reason: Some(reason),
                error_message: message,
                payer: None,
                transaction: None,
                network,
                extensions: None,
            },
        }
    }
}

impl TryFrom<SettleResponseWire> for SettleResponse {
    type Error = String;

    fn try_from(
        wire: SettleResponseWire,
    ) -> Result<Self, <Self as TryFrom<SettleResponseWire>>::Error> {
        if wire.success {
            let payer = wire.payer.ok_or("missing field: payer")?;
            let transaction = wire.transaction.ok_or("missing field: transaction")?;
            Ok(Self::Success {
                payer,
                transaction,
                network: wire.network,
                extensions: wire.extensions,
            })
        } else {
            let reason = wire.error_reason.ok_or("missing field: errorReason")?;
            Ok(Self::Error {
                reason,
                message: wire.error_message,
                network: wire.network,
            })
        }
    }
}

/// A payment required response that can be either V1 or V2.
///
/// This is returned with HTTP 402 status to indicate that payment is required.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PaymentRequired {
    /// Protocol version 1 variant.
    V1(v1::PaymentRequired),
    /// Protocol version 2 variant.
    V2(v2::PaymentRequired),
}
