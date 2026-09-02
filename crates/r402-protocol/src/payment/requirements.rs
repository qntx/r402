//! Seller payment terms inside `PaymentRequired.accepts[]`.

use std::str::FromStr;

use compact_str::CompactString;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::network::ChainId;

/// Payment terms set by the seller.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct PaymentRequirements<
    TScheme = CompactString,
    TAmount = CompactString,
    TAddress = CompactString,
    TExtra = serde_json::Value,
> {
    /// Payment scheme, e.g. `"exact"` or `"upto"`.
    pub scheme: TScheme,
    /// CAIP-2 chain identifier (e.g. `"eip155:8453"`).
    pub network: ChainId,
    /// Payment amount in the token's smallest unit (string for precision).
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

/// First entry in `available` that matches `accepted`.
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
    /// Constructs the six required wire fields.
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

    /// Attaches the scheme-specific `extra` blob.
    #[must_use]
    pub fn with_extra(mut self, extra: TExtra) -> Self {
        self.extra = Some(extra);
        self
    }

    /// Passes through an optional `extra` blob.
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
    /// Converts all-string wire requirements into a typed variant.
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
#[allow(clippy::unwrap_used, reason = "unit tests panic on assertion failure")]
mod tests {
    use super::*;

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
