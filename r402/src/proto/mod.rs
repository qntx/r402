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
use crate::scheme::SchemeHandlerSlug;

pub mod v1;
pub mod v2;

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
    /// Delegates to the same logic as [`VerifyRequest::scheme_handler_slug`].
    #[must_use]
    pub fn scheme_handler_slug(
        &self,
        registry: &crate::networks::NetworkRegistry,
    ) -> Option<SchemeHandlerSlug> {
        // Reuse VerifyRequest's implementation via a temporary reference-based parse.
        let tmp = VerifyRequest(self.0.clone());
        tmp.scheme_handler_slug(registry)
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
    pub fn scheme_handler_slug(
        &self,
        registry: &crate::networks::NetworkRegistry,
    ) -> Option<SchemeHandlerSlug> {
        let x402_version: u8 = self.0.get("x402Version")?.as_u64()?.try_into().ok()?;
        match x402_version {
            v1::X402Version1::VALUE => {
                let network_name = self.0.get("paymentPayload")?.get("network")?.as_str()?;
                let chain_id = registry.chain_id_by_name(network_name)?;
                let scheme = self.0.get("paymentPayload")?.get("scheme")?.as_str()?;
                let slug = SchemeHandlerSlug::new(chain_id.clone(), 1, scheme.into());
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
                let slug = SchemeHandlerSlug::new(chain_id, 2, scheme.into());
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
#[derive(Debug, Clone)]
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

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerifyResponseWire {
    is_valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    payer: Option<String>,
    #[serde(default)]
    invalid_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invalid_message: Option<String>,
}

impl Serialize for VerifyResponse {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let wire = match self {
            Self::Valid { payer } => VerifyResponseWire {
                is_valid: true,
                payer: Some(payer.clone()),
                invalid_reason: None,
                invalid_message: None,
            },
            Self::Invalid {
                reason,
                message,
                payer,
            } => VerifyResponseWire {
                is_valid: false,
                payer: payer.clone(),
                invalid_reason: Some(reason.clone()),
                invalid_message: message.clone(),
            },
        };
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for VerifyResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = VerifyResponseWire::deserialize(deserializer)?;
        if wire.is_valid {
            let payer = wire
                .payer
                .ok_or_else(|| serde::de::Error::missing_field("payer"))?;
            Ok(Self::Valid { payer })
        } else {
            let reason = wire
                .invalid_reason
                .ok_or_else(|| serde::de::Error::missing_field("invalidReason"))?;
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
#[derive(Debug, Clone)]
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

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettleResponseWire {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction: Option<String>,
    pub network: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Extensions>,
}

impl Serialize for SettleResponse {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let wire = match self {
            Self::Success {
                payer,
                transaction,
                network,
                extensions,
            } => SettleResponseWire {
                success: true,
                error_reason: None,
                error_message: None,
                payer: Some(payer.clone()),
                transaction: Some(transaction.clone()),
                network: network.clone(),
                extensions: extensions.clone(),
            },
            Self::Error {
                reason,
                message,
                network,
            } => SettleResponseWire {
                success: false,
                error_reason: Some(reason.clone()),
                error_message: message.clone(),
                payer: None,
                transaction: None,
                network: network.clone(),
                extensions: None,
            },
        };
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SettleResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SettleResponseWire::deserialize(deserializer)?;
        if wire.success {
            let payer = wire
                .payer
                .ok_or_else(|| serde::de::Error::missing_field("payer"))?;
            let transaction = wire
                .transaction
                .ok_or_else(|| serde::de::Error::missing_field("transaction"))?;
            Ok(Self::Success {
                payer,
                transaction,
                network: wire.network,
                extensions: wire.extensions,
            })
        } else {
            let reason = wire
                .error_reason
                .ok_or_else(|| serde::de::Error::missing_field("errorReason"))?;
            Ok(Self::Error {
                reason,
                message: wire.error_message,
                network: wire.network,
            })
        }
    }
}

/// Errors that can occur during payment verification.
///
/// These errors are returned when a payment fails validation checks
/// performed by the facilitator before settlement.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PaymentVerificationError {
    /// The payment payload format is invalid or malformed.
    #[error("Invalid format: {0}")]
    InvalidFormat(String),
    /// The payment amount doesn't match the requirements.
    #[error("Payment amount is invalid with respect to the payment requirements")]
    InvalidPaymentAmount,
    /// The payment authorization's `validAfter` timestamp is in the future.
    #[error("Payment authorization is not yet valid")]
    Early,
    /// The payment authorization's `validBefore` timestamp has passed.
    #[error("Payment authorization is expired")]
    Expired,
    /// The payment's chain ID doesn't match the requirements.
    #[error("Payment chain id is invalid with respect to the payment requirements")]
    ChainIdMismatch,
    /// The payment recipient doesn't match the requirements.
    #[error("Payment recipient is invalid with respect to the payment requirements")]
    RecipientMismatch,
    /// The payment asset (token) doesn't match the requirements.
    #[error("Payment asset is invalid with respect to the payment requirements")]
    AssetMismatch,
    /// The payer's on-chain balance is insufficient.
    #[error("Onchain balance is not enough to cover the payment amount")]
    InsufficientFunds,
    /// The payment signature is invalid.
    #[error("{0}")]
    InvalidSignature(String),
    /// Transaction simulation failed.
    #[error("{0}")]
    TransactionSimulation(String),
    /// The chain is not supported by this facilitator.
    #[error("Unsupported chain")]
    UnsupportedChain,
    /// The payment scheme is not supported by this facilitator.
    #[error("Unsupported scheme")]
    UnsupportedScheme,
    /// The accepted payment details don't match the requirements.
    #[error("Accepted does not match payment requirements")]
    AcceptedRequirementsMismatch,
}

impl AsPaymentProblem for PaymentVerificationError {
    fn as_payment_problem(&self) -> PaymentProblem {
        let error_reason = match self {
            Self::InvalidFormat(_) => ErrorReason::InvalidFormat,
            Self::InvalidPaymentAmount => ErrorReason::InvalidPaymentAmount,
            Self::InsufficientFunds => ErrorReason::InsufficientFunds,
            Self::Early => ErrorReason::InvalidPaymentEarly,
            Self::Expired => ErrorReason::InvalidPaymentExpired,
            Self::ChainIdMismatch => ErrorReason::ChainIdMismatch,
            Self::RecipientMismatch => ErrorReason::RecipientMismatch,
            Self::AssetMismatch => ErrorReason::AssetMismatch,
            Self::InvalidSignature(_) => ErrorReason::InvalidSignature,
            Self::TransactionSimulation(_) => ErrorReason::TransactionSimulation,
            Self::UnsupportedChain => ErrorReason::UnsupportedChain,
            Self::UnsupportedScheme => ErrorReason::UnsupportedScheme,
            Self::AcceptedRequirementsMismatch => ErrorReason::AcceptedRequirementsMismatch,
        };
        PaymentProblem::new(error_reason, self.to_string())
    }
}

impl From<serde_json::Error> for PaymentVerificationError {
    fn from(value: serde_json::Error) -> Self {
        Self::InvalidFormat(value.to_string())
    }
}

/// Machine-readable error reason codes for payment failures.
///
/// These codes are used in error responses to allow clients to
/// programmatically handle different failure scenarios.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorReason {
    /// The payment payload format is invalid.
    InvalidFormat,
    /// The payment amount is incorrect.
    InvalidPaymentAmount,
    /// The payment authorization is not yet valid.
    InvalidPaymentEarly,
    /// The payment authorization has expired.
    InvalidPaymentExpired,
    /// The chain ID doesn't match.
    ChainIdMismatch,
    /// The recipient address doesn't match.
    RecipientMismatch,
    /// The token asset doesn't match.
    AssetMismatch,
    /// The accepted details don't match requirements.
    AcceptedRequirementsMismatch,
    /// The signature is invalid.
    InvalidSignature,
    /// Transaction simulation failed.
    TransactionSimulation,
    /// Insufficient on-chain balance.
    InsufficientFunds,
    /// The chain is not supported.
    UnsupportedChain,
    /// The scheme is not supported.
    UnsupportedScheme,
    /// An unexpected error occurred.
    UnexpectedError,
}

/// Trait for converting errors into structured payment problems.
pub trait AsPaymentProblem {
    /// Converts this error into a [`PaymentProblem`].
    fn as_payment_problem(&self) -> PaymentProblem;
}

/// A structured payment error with reason code and details.
///
/// This type is used to return detailed error information to clients
/// when a payment fails verification or settlement.
#[derive(Debug)]
pub struct PaymentProblem {
    /// The machine-readable error reason.
    reason: ErrorReason,
    /// Human-readable error details.
    details: String,
}

impl PaymentProblem {
    /// Creates a new payment problem with the given reason and details.
    #[must_use]
    pub const fn new(reason: ErrorReason, details: String) -> Self {
        Self { reason, details }
    }

    /// Returns the error reason code.
    #[must_use]
    pub const fn reason(&self) -> ErrorReason {
        self.reason
    }

    /// Returns the human-readable error details.
    #[must_use]
    pub fn details(&self) -> &str {
        &self.details
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
