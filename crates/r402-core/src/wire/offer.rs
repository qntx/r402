//! Seller offer and buyer authorization envelopes.
//!
//! A 402 challenge is [`PaymentRequired`]: resource metadata plus the
//! [`PaymentRequirements`] the seller will accept. The buyer answers with a
//! [`PaymentPayload`]. [`PriceTag`] is the seller-side container for those
//! requirements; scheme `extra` enrichment runs on
//! [`crate::SchemeNetworkServer::enrich_payment_required_response`].

use std::str::FromStr;

use compact_str::CompactString;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::{Extensions, V2, Version2};
use crate::chain::ChainId;

/// Human-readable metadata describing the paid resource.
///
/// Per the x402 v2 spec §5.1.2, only `url` is required. `description` and
/// `mimeType` are optional because many resources (e.g. raw API endpoints)
/// have no meaningful MIME type or prose description. `serviceName`,
/// `tags`, and `iconUrl` are optional discovery metadata consumed by
/// marketplace/bazaar-style aggregators.
///
/// The field names use `camelCase` to align with the wire format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct ResourceInfo {
    /// Canonical URL of the resource.
    pub url: CompactString,
    /// Optional human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<CompactString>,
    /// Optional MIME type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<CompactString>,
    /// Human-readable name of the service hosting the resource.
    ///
    /// Printable ASCII, max 32 characters per spec §5.1.2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_name: Option<CompactString>,
    /// Topical tags for the service, used for discovery filtering.
    ///
    /// Max 5 entries; each printable ASCII, max 32 characters, per spec §5.1.2.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<CompactString>,
    /// Absolute `https`/`http` URL to an icon representing the service.
    ///
    /// Max 2048 characters per spec §5.1.2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<CompactString>,
}

impl ResourceInfo {
    /// Constructs a [`ResourceInfo`] carrying just a URL.
    #[must_use]
    pub fn new(url: impl Into<CompactString>) -> Self {
        Self {
            url: url.into(),
            description: None,
            mime_type: None,
            service_name: None,
            tags: Vec::new(),
            icon_url: None,
        }
    }

    /// Builder: sets `description`.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<CompactString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Builder: sets `mimeType`.
    #[must_use]
    pub fn with_mime_type(mut self, mime_type: impl Into<CompactString>) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }

    /// Builder: sets `serviceName`.
    #[must_use]
    pub fn with_service_name(mut self, service_name: impl Into<CompactString>) -> Self {
        self.service_name = Some(service_name.into());
        self
    }

    /// Builder: replaces the `tags` list.
    #[must_use]
    pub fn with_tags(mut self, tags: Vec<CompactString>) -> Self {
        self.tags = tags;
        self
    }

    /// Builder: appends a single tag.
    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<CompactString>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Builder: sets `iconUrl`.
    #[must_use]
    pub fn with_icon_url(mut self, icon_url: impl Into<CompactString>) -> Self {
        self.icon_url = Some(icon_url.into());
        self
    }
}

#[cfg(test)]
mod resource_info_tests {
    use super::*;

    #[test]
    fn minimal_resource_omits_optional_fields() {
        let info = ResourceInfo::new("https://example.com/paid");
        let v = serde_json::to_value(&info).unwrap();
        assert_eq!(v["url"], "https://example.com/paid");
        assert!(v.get("description").is_none());
        assert!(v.get("mimeType").is_none());
    }

    #[test]
    fn full_resource_roundtrips() {
        let info = ResourceInfo::new("https://example.com/r")
            .with_description("doc")
            .with_mime_type("application/json")
            .with_service_name("Example Weather")
            .with_tag("weather")
            .with_tag("forecast")
            .with_icon_url("https://example.com/icon.png");
        let encoded = serde_json::to_value(&info).unwrap();
        assert_eq!(encoded["mimeType"], "application/json");
        assert_eq!(encoded["serviceName"], "Example Weather");
        assert_eq!(encoded["tags"], serde_json::json!(["weather", "forecast"]));
        assert_eq!(encoded["iconUrl"], "https://example.com/icon.png");
        let decoded: ResourceInfo = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, info);
    }

    /// Spec §5.1.2 discovery metadata is optional; omitting it keeps the
    /// wire payload minimal for resources that don't opt into cataloging.
    #[test]
    fn discovery_metadata_omitted_by_default() {
        let info = ResourceInfo::new("https://example.com/r");
        let v = serde_json::to_value(&info).unwrap();
        assert!(v.get("serviceName").is_none());
        assert!(v.get("tags").is_none());
        assert!(v.get("iconUrl").is_none());
    }

    #[test]
    fn deserializes_spec_compliant_optional_fields() {
        let json = serde_json::json!({ "url": "https://x.test" });
        let decoded: ResourceInfo = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.url, "https://x.test");
        assert!(decoded.description.is_none());
        assert!(decoded.mime_type.is_none());
    }

    /// F-001 regression: unknown top-level field is rejected.
    #[test]
    fn rejects_unknown_field() {
        let json = serde_json::json!({ "url": "https://x.test", "unknown": 1 });
        assert!(serde_json::from_value::<ResourceInfo>(json).is_err());
    }
}

/// Payment terms set by the seller, carried inside `PaymentRequired.accepts[]`.
///
/// Generic parameters allow concrete chain crates to specialise the scheme
/// name, amount representation, addresses, and scheme-specific `extra` blob.
/// The wire-level defaults are all strings plus an opaque JSON value for
/// `extra`, mirroring the protocol exactly.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct PaymentRequirements<
    TScheme = CompactString,
    TAmount = CompactString,
    TAddress = CompactString,
    TExtra = serde_json::Value,
> {
    /// The payment scheme, e.g. `"exact"` or `"upto"`.
    pub scheme: TScheme,
    /// CAIP-2 chain identifier (e.g. `"eip155:8453"`).
    pub network: ChainId,
    /// Payment amount in the token's smallest unit (as string for precision).
    pub amount: TAmount,
    /// Recipient address on the target chain.
    pub pay_to: TAddress,
    /// Maximum time in seconds the authorization remains valid.
    pub max_timeout_seconds: u64,
    /// Token asset address / mint.
    pub asset: TAddress,
    /// Scheme-specific auxiliary data.
    #[serde(default = "Option::default", skip_serializing_if = "Option::is_none")]
    pub extra: Option<TExtra>,
}

/// Finds the first entry in `available` that matches `accepted`
/// ([`PaymentRequirements::matches_payload_accepted`]).
#[must_use]
pub fn find_matching_requirements<'a>(
    available: &'a [PaymentRequirements],
    accepted: &PaymentRequirements,
) -> Option<&'a PaymentRequirements> {
    available
        .iter()
        .find(|req| req.matches_payload_accepted(accepted))
}

impl<TScheme, TAmount, TAddress, TExtra> PaymentRequirements<TScheme, TAmount, TAddress, TExtra> {
    /// Constructs the requirements from the six required wire fields.
    /// Use [`Self::with_extra`] / [`Self::with_optional_extra`] to attach
    /// the scheme-specific blob.
    #[must_use]
    pub const fn new(
        scheme: TScheme,
        network: ChainId,
        amount: TAmount,
        pay_to: TAddress,
        asset: TAddress,
        max_timeout_seconds: u64,
    ) -> Self {
        Self {
            scheme,
            network,
            amount,
            pay_to,
            asset,
            max_timeout_seconds,
            extra: None,
        }
    }

    /// Builder: attaches the scheme-specific `extra` blob.
    #[must_use]
    pub fn with_extra(mut self, extra: TExtra) -> Self {
        self.extra = Some(extra);
        self
    }

    /// Builder: passes through an optional `extra` blob (useful when the
    /// value is produced via `Option::map` upstream).
    #[must_use]
    pub fn with_optional_extra(mut self, extra: Option<TExtra>) -> Self {
        self.extra = extra;
        self
    }
}

impl<TScheme, TAmount, TAddress, TExtra> PaymentRequirements<TScheme, TAmount, TAddress, TExtra>
where
    TScheme: PartialEq,
    TAmount: PartialEq,
    TAddress: PartialEq,
    TExtra: Serialize,
{
    /// Core fields including `maxTimeoutSeconds` must be equal.
    /// Server-declared `extra` is a subset of `accepted.extra`.
    #[must_use]
    pub fn matches_payload_accepted(&self, accepted: &Self) -> bool {
        self.matches_payload_accepted_with_dynamic(accepted, &[])
    }

    /// Like [`Self::matches_payload_accepted`], omitting `dynamic_extra_fields`
    /// from both extras before the subset check.
    #[must_use]
    pub fn matches_payload_accepted_with_dynamic(
        &self,
        accepted: &Self,
        dynamic_extra_fields: &[&str],
    ) -> bool {
        self.scheme == accepted.scheme
            && self.network == accepted.network
            && self.amount == accepted.amount
            && self.asset == accepted.asset
            && self.pay_to == accepted.pay_to
            && self.max_timeout_seconds == accepted.max_timeout_seconds
            && extra_contains_subset(
                self.extra.as_ref(),
                accepted.extra.as_ref(),
                dynamic_extra_fields,
            )
    }
}

fn extra_contains_subset<T: Serialize>(
    required: Option<&T>,
    accepted: Option<&T>,
    dynamic_extra_fields: &[&str],
) -> bool {
    let Some(required_extra) = required else {
        return true;
    };
    let Ok(required_value) = serde_json::to_value(required_extra) else {
        return false;
    };
    let accepted_value = accepted.and_then(|extra| serde_json::to_value(extra).ok());
    if accepted.is_some() && accepted_value.is_none() {
        return false;
    }
    let required_omitted = omit_fields(&required_value, dynamic_extra_fields);
    let accepted_omitted = accepted_value
        .as_ref()
        .map(|value| omit_fields(value, dynamic_extra_fields));
    object_contains_subset(&required_omitted, accepted_omitted.as_ref())
}

fn omit_fields(value: &serde_json::Value, fields: &[&str]) -> serde_json::Value {
    if fields.is_empty() {
        return value.clone();
    }
    let serde_json::Value::Object(map) = value else {
        return value.clone();
    };
    let mut copied = map.clone();
    for field in fields {
        copied.remove(*field);
    }
    serde_json::Value::Object(copied)
}

/// Missing object keys match only when the required value is JSON `null`.
fn object_contains_subset(
    expected: &serde_json::Value,
    actual: Option<&serde_json::Value>,
) -> bool {
    let serde_json::Value::Object(expected_map) = expected else {
        return actual.is_some_and(|got| got == expected);
    };
    let Some(serde_json::Value::Object(actual_map)) = actual else {
        return false;
    };
    expected_map.iter().all(|(key, value)| {
        actual_map.get(key).map_or_else(
            || value.is_null(),
            |got| object_contains_subset(value, Some(got)),
        )
    })
}

impl PaymentRequirements {
    /// Attempts to convert the wire-level requirements (all-strings) into
    /// a concrete, strongly-typed variant.
    ///
    /// Returns `None` if any component fails to parse.
    #[must_use]
    pub fn as_concrete<TScheme, TAmount, TAddress, TExtra>(
        &self,
    ) -> Option<PaymentRequirements<TScheme, TAmount, TAddress, TExtra>>
    where
        TScheme: FromStr,
        TAmount: FromStr,
        TAddress: FromStr,
        TExtra: DeserializeOwned,
    {
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

#[cfg(test)]
mod payment_requirements_tests {
    use super::*;

    /// F-001 regression: typo in a top-level field is rejected at parse time
    /// rather than silently ignored, leading to clearer diagnostics.
    #[test]
    fn rejects_unknown_top_level_field() {
        let json = serde_json::json!({
            "scheme": "exact",
            "network": "eip155:8453",
            "amount": "1",
            "payTo": "0x0",
            "maxTimeoutSeconds": 60,
            "asset": "0x0",
            "unknownField": 1
        });
        assert!(serde_json::from_value::<PaymentRequirements>(json).is_err());
    }

    fn sample(network: &str, amount: &str, pay_to: &str, timeout: u64) -> PaymentRequirements {
        PaymentRequirements::new(
            "exact".into(),
            network.parse().unwrap(),
            amount.into(),
            pay_to.into(),
            "USDC".into(),
            timeout,
        )
    }

    #[test]
    fn find_matching_requirements_go_semantics() {
        let a = sample("eip155:1", "1000000", "0xrecipient1", 60);
        let b = sample("eip155:8453", "2000000", "0xrecipient2", 30);
        let available = [a.clone(), b.clone()];

        let matched = find_matching_requirements(&available, &b).unwrap();
        assert_eq!(matched.network.to_string(), "eip155:8453");
        assert_eq!(matched.max_timeout_seconds, 30);

        let mut timeout_miss = b;
        timeout_miss.max_timeout_seconds = 999;
        assert!(find_matching_requirements(&available, &timeout_miss).is_none());

        let mut miss = a;
        miss.scheme = "nonexistent".into();
        assert!(find_matching_requirements(&available, &miss).is_none());
    }

    #[test]
    fn matches_when_accepted_extra_has_additional_object_keys() {
        let required =
            sample("eip155:8453", "1000000", "0xabc", 300).with_extra(serde_json::json!({
                "name": "USDC",
                "version": "2",
                "nested": { "required": true }
            }));
        let accepted =
            sample("eip155:8453", "1000000", "0xabc", 300).with_extra(serde_json::json!({
                "name": "USDC",
                "version": "2",
                "nested": { "required": true, "clientOnly": "ok" },
                "channelState": { "chargedCumulativeAmount": "2000" }
            }));
        assert!(required.matches_payload_accepted(&accepted));
    }

    #[test]
    fn matches_when_required_extra_is_absent() {
        let required = sample("eip155:8453", "1000000", "0xabc", 300);
        let accepted = sample("eip155:8453", "1000000", "0xabc", 300)
            .with_extra(serde_json::json!({ "clientOnly": true }));
        assert!(required.matches_payload_accepted(&accepted));
    }

    #[test]
    fn matches_when_required_null_key_is_missing_on_accepted() {
        let required = sample("eip155:8453", "1000000", "0xabc", 300)
            .with_extra(serde_json::json!({ "k": null }));
        let accepted =
            sample("eip155:8453", "1000000", "0xabc", 300).with_extra(serde_json::json!({}));
        assert!(required.matches_payload_accepted(&accepted));
    }

    #[test]
    fn does_not_match_when_accepted_extra_overwrites_server_field() {
        let required = sample("eip155:8453", "1000000", "0xabc", 300)
            .with_extra(serde_json::json!({ "name": "USDC", "version": "2" }));
        let accepted = sample("eip155:8453", "1000000", "0xabc", 300)
            .with_extra(serde_json::json!({ "name": "USDC", "version": "3" }));
        assert!(!required.matches_payload_accepted(&accepted));
    }

    #[test]
    fn does_not_match_when_accepted_extra_array_is_a_superset() {
        let required = sample("eip155:8453", "1000000", "0xabc", 300)
            .with_extra(serde_json::json!({ "allowedSigners": ["0xalice"] }));
        let accepted = sample("eip155:8453", "1000000", "0xabc", 300)
            .with_extra(serde_json::json!({ "allowedSigners": ["0xmallory", "0xalice"] }));
        assert!(!required.matches_payload_accepted(&accepted));
    }

    #[test]
    fn does_not_match_when_accepted_extra_array_is_reordered() {
        let required = sample("eip155:8453", "1000000", "0xabc", 300)
            .with_extra(serde_json::json!({ "allowedSigners": ["0xalice", "0xbob"] }));
        let accepted = sample("eip155:8453", "1000000", "0xabc", 300)
            .with_extra(serde_json::json!({ "allowedSigners": ["0xbob", "0xalice"] }));
        assert!(!required.matches_payload_accepted(&accepted));
    }

    #[test]
    fn does_not_match_when_accepted_extra_omits_server_fields() {
        let required = sample("eip155:8453", "1000000", "0xabc", 300)
            .with_extra(serde_json::json!({ "name": "USDC", "version": "2" }));
        let accepted = sample("eip155:8453", "1000000", "0xabc", 300)
            .with_extra(serde_json::json!({ "name": "USDC" }));
        assert!(!required.matches_payload_accepted(&accepted));
    }

    #[test]
    fn matches_when_only_declared_dynamic_extra_fields_differ() {
        let required =
            sample("solana:mainnet", "1000000", "PayTo1", 60).with_extra(serde_json::json!({
                "feePayer": "FeePayer111111111111111111111111111111111",
                "recentBlockhash": "freshBlockhash",
                "lastValidBlockHeight": "200"
            }));
        let accepted =
            sample("solana:mainnet", "1000000", "PayTo1", 60).with_extra(serde_json::json!({
                "feePayer": "FeePayer111111111111111111111111111111111",
                "recentBlockhash": "staleBlockhash",
                "lastValidBlockHeight": "100"
            }));
        assert!(required.matches_payload_accepted_with_dynamic(
            &accepted,
            &["recentBlockhash", "lastValidBlockHeight"],
        ));
        assert!(!required.matches_payload_accepted(&accepted));
    }

    #[test]
    fn does_not_match_when_static_extra_differs_despite_dynamic_fields() {
        let required =
            sample("solana:mainnet", "1000000", "PayTo1", 60).with_extra(serde_json::json!({
                "feePayer": "FeePayer111111111111111111111111111111111",
                "recentBlockhash": "freshBlockhash"
            }));
        let accepted =
            sample("solana:mainnet", "1000000", "PayTo1", 60).with_extra(serde_json::json!({
                "feePayer": "OtherPayer1111111111111111111111111111111",
                "recentBlockhash": "staleBlockhash"
            }));
        assert!(!required.matches_payload_accepted_with_dynamic(
            &accepted,
            &["recentBlockhash", "lastValidBlockHeight"],
        ));
    }
}

/// Body of an HTTP 402 "Payment Required" response.
///
/// Contains:
///
/// - the x402 version marker,
/// - an optional human-readable `error` string for malformed clients,
/// - resource metadata,
/// - the list of [`PaymentRequirements`] the seller will accept, and
/// - an optional `extensions` block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct PaymentRequired {
    /// Protocol version (always `2`).
    pub x402_version: Version2,
    /// Optional error message describing why the request was rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<CompactString>,
    /// Resource metadata.
    pub resource: ResourceInfo,
    /// Accepted payment terms.
    #[serde(default)]
    pub accepts: Vec<PaymentRequirements>,
    /// Optional extension block.
    #[serde(default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl PaymentRequired {
    /// Constructs a 402 body with the resource block and an empty
    /// `accepts` list. Use [`Self::with_accepts`] to attach payment
    /// requirements and [`Self::with_error`] to surface a diagnostic
    /// message to malformed clients.
    #[must_use]
    pub fn new(resource: ResourceInfo) -> Self {
        Self {
            x402_version: V2,
            error: None,
            resource,
            accepts: Vec::new(),
            extensions: Extensions::new(),
        }
    }

    /// Builder: replaces the accepted payment requirements list.
    #[must_use]
    pub fn with_accepts(mut self, accepts: Vec<PaymentRequirements>) -> Self {
        self.accepts = accepts;
        self
    }

    /// Builder: appends a single payment requirement to the `accepts` list.
    #[must_use]
    pub fn add_accept(mut self, accept: PaymentRequirements) -> Self {
        self.accepts.push(accept);
        self
    }

    /// Builder: attaches a human-readable error message describing why the
    /// request was rejected (e.g. `"missing X-PAYMENT header"`).
    #[must_use]
    pub fn with_error(mut self, error: impl Into<CompactString>) -> Self {
        self.error = Some(error.into());
        self
    }

    /// Builder: replaces the `extensions` block.
    #[must_use]
    pub fn with_extensions(mut self, extensions: Extensions) -> Self {
        self.extensions = extensions;
        self
    }
}

/// A signed payment authorization sent by the buyer to the seller.
///
/// In x402 v2 the payload is self-describing: it carries the `accepted`
/// requirements the buyer chose (so the facilitator can re-verify them)
/// plus the scheme-specific `payload`, an optional resource descriptor,
/// and an optional `extensions` map.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct PaymentPayload<TAccepted, TPayload> {
    /// The terms the buyer accepted (a full [`PaymentRequirements`] form).
    pub accepted: TAccepted,
    /// Scheme-specific signed payload (e.g., EIP-3009 authorization).
    pub payload: TPayload,
    /// Optional resource metadata copied from the 402 response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceInfo>,
    /// Protocol version marker (always `2`).
    pub x402_version: Version2,
    /// Optional extension payload block.
    #[serde(default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl<TAccepted, TPayload> PaymentPayload<TAccepted, TPayload> {
    /// Constructs a payload from the two required fields. Use the
    /// [`Self::with_resource`] / [`Self::with_extensions`] builders to
    /// attach the optional blocks.
    #[must_use]
    pub fn new(accepted: TAccepted, payload: TPayload) -> Self {
        Self {
            accepted,
            payload,
            resource: None,
            x402_version: V2,
            extensions: Extensions::new(),
        }
    }

    /// Builder: attaches optional resource metadata.
    #[must_use]
    pub fn with_resource(mut self, resource: ResourceInfo) -> Self {
        self.resource = Some(resource);
        self
    }

    /// Builder: attaches an optional resource (passes through `None`
    /// untouched, useful when the value is produced via `Option::map`).
    #[must_use]
    pub fn with_optional_resource(mut self, resource: Option<ResourceInfo>) -> Self {
        self.resource = resource;
        self
    }

    /// Builder: replaces the `extensions` block.
    #[must_use]
    pub fn with_extensions(mut self, extensions: Extensions) -> Self {
        self.extensions = extensions;
        self
    }
}

/// Seller-side payment-requirements container.
///
/// Constructed from a price + chain + asset. Scheme-specific `extra`
/// (fee payer, facilitator address, …) is filled by
/// [`crate::SchemeNetworkServer::enrich_payment_required_response`] during
/// 402 construction — not on this type.
#[derive(Debug, Clone)]
pub struct PriceTag {
    /// The requirements being advertised.
    pub requirements: PaymentRequirements,
}

impl PriceTag {
    /// Constructs a price tag around existing requirements.
    #[must_use]
    pub const fn new(requirements: PaymentRequirements) -> Self {
        Self { requirements }
    }

    /// Overrides the `maxTimeoutSeconds` field.
    #[must_use]
    pub const fn with_timeout(mut self, seconds: u64) -> Self {
        self.requirements.max_timeout_seconds = seconds;
        self
    }
}
