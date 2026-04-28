//! Seller-declared payment terms.

use std::str::FromStr;

use compact_str::CompactString;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::chain::ChainId;

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
mod tests {
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
}
