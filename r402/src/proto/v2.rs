//! Protocol version 2 (V2) types for x402.
//!
//! This module defines the wire format types for the enhanced x402 protocol version.
//! V2 uses CAIP-2 chain IDs (e.g., "eip155:8453") instead of network names, and
//! includes richer resource metadata.
//!
//! # Key Differences from V1
//!
//! - Uses CAIP-2 chain IDs instead of network names
//! - Includes [`ResourceInfo`] with URL, description, and MIME type
//! - Simplified [`PaymentRequirements`] structure
//! - Payment payload includes accepted requirements for verification
//!
//! # Key Types
//!
//! - [`X402Version2`] - Version marker that serializes as `2`
//! - [`PaymentPayload`] - Signed payment with accepted requirements
//! - [`PaymentRequirements`] - Payment terms set by the seller
//! - [`PaymentRequired`] - HTTP 402 response body
//! - [`ResourceInfo`] - Metadata about the paid resource
//! - [`PriceTag`] - Builder for creating payment requirements

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::chain::ChainId;
use crate::proto;
use crate::proto::SupportedResponse;

/// Version marker for x402 protocol version 2.
///
/// This is a type alias for [`super::Version<2>`] that serializes as the
/// integer `2` and rejects other values on deserialization.
///
/// Use the [`V2`] constant when constructing V2 protocol messages.
pub type X402Version2 = super::Version<2>;

/// Convenience constant for constructing V2 protocol messages.
pub const V2: X402Version2 = super::Version;

/// Response from a V2 payment verification request.
///
/// V2 uses the same response format as the protocol-level type.
pub type VerifyResponse = proto::VerifyResponse;

/// Response from a V2 payment settlement request.
///
/// V2 uses the same response format as the protocol-level type.
pub type SettleResponse = proto::SettleResponse;

/// Metadata about the resource being paid for.
///
/// This provides human-readable information about what the buyer is paying for.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceInfo {
    /// Human-readable description of the resource.
    pub description: String,
    /// MIME type of the resource content.
    pub mime_type: String,
    /// URL of the resource.
    pub url: String,
}

/// Request to verify a V2 payment.
///
/// Contains the payment payload and requirements for verification.
/// This is a type alias for [`proto::TypedVerifyRequest`] with version 2.
pub type VerifyRequest<TPayload, TRequirements> =
    proto::TypedVerifyRequest<2, TPayload, TRequirements>;

/// A signed payment authorization from the buyer (V2 format).
///
/// In V2, the payment payload includes the accepted requirements, allowing
/// the facilitator to verify that the buyer agreed to specific terms.
///
/// # Type Parameters
///
/// - `TAccepted` - The accepted requirements type
/// - `TPayload` - The scheme-specific payload type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentPayload<TAccepted, TPayload> {
    /// The payment requirements the buyer accepted.
    pub accepted: TAccepted,
    /// The scheme-specific signed payload.
    pub payload: TPayload,
    /// Information about the resource being paid for.
    pub resource: Option<ResourceInfo>,
    /// Protocol version (always 2).
    pub x402_version: X402Version2,
    /// Optional protocol extensions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<proto::Extensions>,
}

/// Payment requirements set by the seller (V2 format).
///
/// Defines the terms under which a payment will be accepted. V2 uses
/// CAIP-2 chain IDs and has a simplified structure compared to V1.
///
/// # Type Parameters
///
/// - `TScheme` - The scheme identifier type (default: `String`)
/// - `TAmount` - The amount type (default: `String`)
/// - `TAddress` - The address type (default: `String`)
/// - `TExtra` - Scheme-specific extra data type (default: `serde_json::Value`)
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequirements<
    TScheme = String,
    TAmount = String,
    TAddress = String,
    TExtra = serde_json::Value,
> {
    /// The payment scheme (e.g., "exact").
    pub scheme: TScheme,
    /// The CAIP-2 chain ID (e.g., "eip155:8453").
    pub network: ChainId,
    /// The payment amount in token units.
    pub amount: TAmount,
    /// The recipient address for payment.
    pub pay_to: TAddress,
    /// Maximum time in seconds for payment validity.
    pub max_timeout_seconds: u64,
    /// The token asset address.
    pub asset: TAddress,
    /// Scheme-specific extra data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<TExtra>,
}

impl PaymentRequirements {
    /// Converts the payment requirements to a concrete type.
    #[must_use]
    pub fn as_concrete<
        TScheme: FromStr,
        TAmount: FromStr,
        TAddress: FromStr,
        TExtra: DeserializeOwned,
    >(
        &self,
    ) -> Option<PaymentRequirements<TScheme, TAmount, TAddress, TExtra>> {
        let scheme = self.scheme.parse::<TScheme>().ok()?;
        let amount = self.amount.parse::<TAmount>().ok()?;
        let pay_to = self.pay_to.parse::<TAddress>().ok()?;
        let asset = self.asset.parse::<TAddress>().ok()?;
        let extra = self
            .extra
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        Some(PaymentRequirements {
            scheme,
            network: self.network.clone(),
            amount,
            pay_to,
            max_timeout_seconds: self.max_timeout_seconds,
            asset,
            extra,
        })
    }
}

/// HTTP 402 Payment Required response body for V2.
///
/// This is returned when a resource requires payment. It contains
/// the list of acceptable payment methods and resource metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequired {
    /// Protocol version (always 2).
    pub x402_version: X402Version2,
    /// Optional error message if the request was malformed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Information about the resource being paid for.
    pub resource: ResourceInfo,
    /// List of acceptable payment methods.
    #[serde(default)]
    pub accepts: Vec<PaymentRequirements>,
    /// Optional protocol extensions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<proto::Extensions>,
}

/// Builder for creating V2 payment requirements.
///
/// A `PriceTag` wraps [`PaymentRequirements`] and provides enrichment
/// capabilities for adding facilitator-specific data.
#[derive(Clone)]
pub struct PriceTag {
    /// The payment requirements.
    pub requirements: PaymentRequirements,
    /// Optional enrichment function for adding facilitator-specific data.
    #[doc(hidden)]
    pub enricher: Option<Enricher>,
}

impl fmt::Debug for PriceTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PriceTag")
            .field("requirements", &self.requirements)
            .field("enricher", &self.enricher.as_ref().map(|_| "<fn>"))
            .finish()
    }
}

/// Enrichment function type for V2 price tags.
///
/// Enrichers are called with the facilitator's capabilities to add
/// facilitator-specific data to price tags (e.g., fee payer addresses).
pub type Enricher = Arc<dyn Fn(&mut PriceTag, &SupportedResponse) + Send + Sync>;

impl PriceTag {
    /// Applies the enrichment function if one is set.
    ///
    /// This is called automatically when building payment requirements
    /// to add facilitator-specific data.
    pub fn enrich(&mut self, capabilities: &SupportedResponse) {
        if let Some(enricher) = self.enricher.clone() {
            enricher(self, capabilities);
        }
    }

    /// Sets the maximum timeout for this price tag.
    #[must_use]
    pub const fn with_timeout(mut self, seconds: u64) -> Self {
        self.requirements.max_timeout_seconds = seconds;
        self
    }
}

/// Compares a [`PriceTag`] with [`PaymentRequirements`] on the five
/// protocol-critical fields only: scheme, network, amount, asset, and `pay_to`.
///
/// This mirrors the Go SDK's `FindMatchingRequirements` which deliberately
/// ignores `max_timeout_seconds` and `extra` to avoid false-negative rejections
/// when facilitator enrichment adds scheme-specific metadata.
impl PartialEq<PaymentRequirements> for PriceTag {
    fn eq(&self, b: &PaymentRequirements) -> bool {
        let a = &self.requirements;
        a.scheme == b.scheme
            && a.network == b.network
            && a.amount == b.amount
            && a.asset == b.asset
            && a.pay_to == b.pay_to
    }
}
