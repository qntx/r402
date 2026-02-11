//! Protocol version 1 (V1) types for x402.
//!
//! This module defines the wire format types for the original x402 protocol version.
//! V1 uses network names (e.g., "base-sepolia") instead of CAIP-2 chain IDs.
//!
//! # Key Types
//!
//! - [`X402Version1`] - Version marker that serializes as `1`
//! - [`PaymentPayload`] - Signed payment authorization from the buyer
//! - [`PaymentRequirements`] - Payment terms set by the seller
//! - [`PaymentRequired`] - HTTP 402 response body
//! - [`VerifyRequest`] / [`VerifyResponse`] - Verification messages
//! - [`SettleResponse`] - Settlement result
//! - [`PriceTag`] - Builder for creating payment requirements

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use crate::proto;
use crate::proto::SupportedResponse;

/// Version marker for x402 protocol version 1.
///
/// This is a type alias for [`super::Version<1>`] that serializes as the
/// integer `1` and rejects other values on deserialization.
///
/// Use the [`V1`] constant when constructing V1 protocol messages.
pub type X402Version1 = super::Version<1>;

/// Convenience constant for constructing V1 protocol messages.
pub const V1: X402Version1 = super::Version;

/// Response from a V1 payment verification request.
///
/// V1 uses the same response format as the protocol-level type.
pub type VerifyResponse = proto::VerifyResponse;

/// Response from a V1 payment settlement request.
///
/// V1 uses the same response format as the protocol-level type.
pub type SettleResponse = proto::SettleResponse;

/// Request to verify a V1 payment.
///
/// Contains the payment payload and requirements for verification.
/// This is a type alias for [`proto::TypedVerifyRequest`] with version 1.
pub type VerifyRequest<TPayload, TRequirements> =
    proto::TypedVerifyRequest<1, TPayload, TRequirements>;

/// A signed payment authorization from the buyer.
///
/// This contains the cryptographic proof that the buyer has authorized
/// a payment, along with metadata about the payment scheme and network.
///
/// # Type Parameters
///
/// - `TScheme` - The scheme identifier type (default: `String`)
/// - `TPayload` - The scheme-specific payload type (default: raw JSON)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentPayload<TScheme = String, TPayload = Box<serde_json::value::RawValue>> {
    /// Protocol version (always 1).
    pub x402_version: X402Version1,
    /// The payment scheme (e.g., "exact").
    pub scheme: TScheme,
    /// The network name (e.g., "base-sepolia").
    pub network: String,
    /// The scheme-specific signed payload.
    pub payload: TPayload,
}

/// Payment requirements set by the seller.
///
/// Defines the terms under which a payment will be accepted, including
/// the amount, recipient, asset, and timing constraints.
///
/// # Type Parameters
///
/// - `TScheme` - The scheme identifier type (default: `String`)
/// - `TAmount` - The amount type (default: `String`)
/// - `TAddress` - The address type (default: `String`)
/// - `TExtra` - Scheme-specific extra data type (default: `serde_json::Value`)
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequirements<
    TScheme = String,
    TAmount = String,
    TAddress = String,
    TExtra = serde_json::Value,
> {
    /// The payment scheme (e.g., "exact").
    pub scheme: TScheme,
    /// The network name (e.g., "base-sepolia").
    pub network: String,
    /// The maximum amount required for payment.
    pub max_amount_required: TAmount,
    /// The resource URL being paid for.
    pub resource: String,
    /// Human-readable description of the resource.
    pub description: String,
    /// MIME type of the resource.
    pub mime_type: String,
    /// Optional JSON schema for the resource output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
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
    ///
    /// Returns `None` if any of the type conversions fail (e.g., parsing scheme,
    /// amount, or address strings into their typed equivalents).
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
        let max_amount_required = self.max_amount_required.parse::<TAmount>().ok()?;
        let pay_to = self.pay_to.parse::<TAddress>().ok()?;
        let asset = self.asset.parse::<TAddress>().ok()?;
        let extra = self
            .extra
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        Some(PaymentRequirements {
            scheme,
            network: self.network.clone(),
            max_amount_required,
            resource: self.resource.clone(),
            description: self.description.clone(),
            mime_type: self.mime_type.clone(),
            output_schema: self.output_schema.clone(),
            pay_to,
            max_timeout_seconds: self.max_timeout_seconds,
            asset,
            extra,
        })
    }
}

/// HTTP 402 Payment Required response body for V1.
///
/// This is returned when a resource requires payment. It contains
/// the list of acceptable payment methods.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequired {
    /// Protocol version (always 1).
    pub x402_version: X402Version1,
    /// List of acceptable payment methods.
    #[serde(default)]
    pub accepts: Vec<PaymentRequirements>,
    /// Optional error message if the request was malformed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Builder for creating payment requirements.
///
/// A `PriceTag` is a convenient way to specify payment terms that can
/// be converted into [`PaymentRequirements`] for inclusion in a 402 response.
#[derive(Clone)]
pub struct PriceTag {
    /// The payment scheme (e.g., "exact").
    pub scheme: String,
    /// The recipient address.
    pub pay_to: String,
    /// The token asset address.
    pub asset: String,
    /// The network name.
    pub network: String,
    /// The payment amount in token units.
    pub amount: String,
    /// Maximum time in seconds for payment validity.
    pub max_timeout_seconds: u64,
    /// Scheme-specific extra data.
    pub extra: Option<serde_json::Value>,
    /// Optional enrichment function for adding facilitator-specific data.
    #[doc(hidden)]
    pub enricher: Option<Enricher>,
}

impl fmt::Debug for PriceTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PriceTag")
            .field("scheme", &self.scheme)
            .field("pay_to", &self.pay_to)
            .field("asset", &self.asset)
            .field("network", &self.network)
            .field("amount", &self.amount)
            .field("max_timeout_seconds", &self.max_timeout_seconds)
            .field("extra", &self.extra)
            .field("enricher", &self.enricher.as_ref().map(|_| "<fn>"))
            .finish()
    }
}

/// Enrichment function type for price tags.
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
        self.max_timeout_seconds = seconds;
        self
    }
}
