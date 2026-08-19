//! HTTP-level support for the x402 **upto** scheme (partial settlement).
//!
//! Official foundation servers bill actual usage via the
//! [`SETTLEMENT_OVERRIDES_HEADER`] (`Settlement-Overrides`) JSON value set by
//! the route handler. Handlers may also use the in-process
//! [`UptoActualAmount`] response extension (atomic units only).
//!
//! # Amount formats ([`SettlementOverrides::amount`])
//!
//! - Raw atomic units: `"1000"`
//! - Percent of authorized max: `"50%"` (up to 2 decimal places, floored)
//! - Dollar price: `"$0.05"` (converted with `extra.decimals`, default 6)
//!
//! # Flow (sequential settlement)
//!
//! ```text
//! client          middleware           handler
//!   |                 |                    |
//!   | POST + sig ---->| verify             |
//!   |                 |---- request ------>|
//!   |                 |                    |  set Settlement-Overrides
//!   |                 |                    |  or UptoActualAmount
//!   |                 |<--- response ------|
//!   |                 | resolve amount
//!   |                 | strip Overrides hdr
//!   |                 | settle(actual)
//!   |<------- resp ---|
//! ```
//!
//! # Settlement modes
//!
//! Only [`SettlementMode::Sequential`](super::SettlementMode::Sequential)
//! honours amount overrides: concurrent and background modes settle the
//! signed maximum before the handler can declare a partial amount.
//!
//! # Example
//!
//! ```ignore
//! use axum::response::IntoResponse;
//! use r402_http::server::{SettlementOverrides, set_settlement_overrides};
//!
//! async fn handler() -> impl IntoResponse {
//!     let mut response = "Hello".into_response();
//!     set_settlement_overrides(&mut response, &SettlementOverrides::amount("50%"));
//!     response
//! }
//! ```

use compact_str::CompactString;
use http::{HeaderMap, HeaderName, HeaderValue};
/// Official partial-settlement overrides (`amount` atomic / percent / dollar).
pub use r402_core::SettlementOverrides;
use r402_core::wire::PaymentRequirements;
use r402_core::wire::{
    SettlementOverrideError, asset_decimals_from_extra, resolve_settlement_override_amount,
};

/// Official HTTP header for partial settlement (`Settlement-Overrides`).
pub const SETTLEMENT_OVERRIDES_HEADER: &str = "Settlement-Overrides";

/// Header name for insertion / removal (lowercase wire form).
#[must_use]
pub const fn settlement_overrides_header_name() -> HeaderName {
    HeaderName::from_static("settlement-overrides")
}

/// Serializes overrides into the `Settlement-Overrides` response header.
///
/// Mirrors Go `SetSettlementOverrides` / TS `setSettlementOverrides`.
pub fn set_settlement_overrides(headers: &mut HeaderMap, overrides: &SettlementOverrides) {
    if let Ok(json) = serde_json::to_string(overrides)
        && let Ok(value) = HeaderValue::from_str(&json)
    {
        headers.insert(settlement_overrides_header_name(), value);
    }
}

/// Convenience when the handler holds a full [`axum_core::response::Response`].
pub fn set_settlement_overrides_on_response(
    response: &mut axum_core::response::Response,
    overrides: &SettlementOverrides,
) {
    set_settlement_overrides(response.headers_mut(), overrides);
}

/// Parses `Settlement-Overrides` from response headers and **removes** it so
/// the client never receives the internal billing header.
///
/// Malformed JSON is ignored (matches Go `ProcessSettlement`).
#[must_use]
pub fn take_settlement_overrides_header(headers: &mut HeaderMap) -> Option<SettlementOverrides> {
    let value = headers.remove(settlement_overrides_header_name())?;
    let s = value.to_str().ok()?;
    serde_json::from_str(s).ok()
}

/// Resolves a handler-declared amount override to atomic units.
///
/// Precedence (mirrors Go: explicit overrides win over the header):
/// 1. [`UptoActualAmount`] response extension (atomic string, no further parse)
/// 2. `Settlement-Overrides` header (atomic / percent / dollar)
///
/// # Errors
///
/// Propagates [`SettlementOverrideError`] when the header amount cannot be
/// resolved against `requirements`.
pub fn resolve_response_settlement_amount(
    response: &mut axum_core::response::Response,
    requirements: &PaymentRequirements,
) -> Result<Option<String>, SettlementOverrideError> {
    if let Some(ext) = response.extensions_mut().remove::<UptoActualAmount>() {
        return Ok(Some(ext.into_inner().into()));
    }

    let Some(overrides) = take_settlement_overrides_header(response.headers_mut()) else {
        return Ok(None);
    };
    let Some(raw) = overrides.amount.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let decimals = asset_decimals_from_extra(requirements.extra.as_ref());
    let resolved =
        resolve_settlement_override_amount(&raw, requirements.amount.as_str(), decimals)?;
    Ok(Some(resolved))
}

/// Response extension instructing the middleware to settle for this atomic amount.
///
/// Prefer [`set_settlement_overrides`] with official amount formats when the
/// handler is HTTP-aware. This extension remains for in-process handlers that
/// already computed atomic units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UptoActualAmount(CompactString);

impl UptoActualAmount {
    /// Creates a new override from a decimal-string amount (atomic units).
    pub fn new<S: Into<CompactString>>(amount: S) -> Self {
        Self(amount.into())
    }

    /// Returns the wrapped amount as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Consumes the wrapper and returns the inner [`CompactString`].
    #[must_use]
    pub fn into_inner(self) -> CompactString {
        self.0
    }
}

impl AsRef<str> for UptoActualAmount {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl From<CompactString> for UptoActualAmount {
    fn from(value: CompactString) -> Self {
        Self(value)
    }
}

impl From<&str> for UptoActualAmount {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

impl From<String> for UptoActualAmount {
    fn from(value: String) -> Self {
        Self(value.into())
    }
}

/// Serializes overrides the same way foundation HTTP adapters do.
#[must_use]
pub fn marshal_settlement_overrides(overrides: &SettlementOverrides) -> String {
    serde_json::to_string(overrides).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use http::HeaderMap;
    use r402_core::wire::PaymentRequirements;

    use super::*;

    #[test]
    fn roundtrips_through_str_constructors() {
        let a = UptoActualAmount::new("125000");
        assert_eq!(a.as_str(), "125000");
        assert_eq!(a.as_ref(), "125000");
        assert_eq!(a.into_inner().as_str(), "125000");
    }

    #[test]
    fn is_constructible_from_string_like() {
        let _a: UptoActualAmount = "1".into();
        let _b: UptoActualAmount = String::from("2").into();
        let _c: UptoActualAmount = CompactString::from("3").into();
    }

    #[test]
    fn header_roundtrip_and_strip() {
        let mut headers = HeaderMap::new();
        set_settlement_overrides(&mut headers, &SettlementOverrides::amount("50%"));
        assert!(headers.contains_key(settlement_overrides_header_name()));
        let parsed = take_settlement_overrides_header(&mut headers).unwrap();
        assert_eq!(parsed.amount.as_deref(), Some("50%"));
        assert!(!headers.contains_key(settlement_overrides_header_name()));
    }

    #[test]
    fn resolve_percent_from_header_on_response() {
        let mut response = axum_core::response::Response::new(axum_core::body::Body::empty());
        set_settlement_overrides_on_response(&mut response, &SettlementOverrides::amount("50%"));
        let req = PaymentRequirements::new(
            "upto".into(),
            "eip155:8453".parse().unwrap(),
            "1000000".into(),
            "0xpay".into(),
            "0xasset".into(),
            300,
        );
        let amount = resolve_response_settlement_amount(&mut response, &req)
            .unwrap()
            .unwrap();
        assert_eq!(amount, "500000");
        assert!(
            !response
                .headers()
                .contains_key(settlement_overrides_header_name())
        );
    }

    #[test]
    fn extension_beats_header() {
        let mut response = axum_core::response::Response::new(axum_core::body::Body::empty());
        set_settlement_overrides_on_response(&mut response, &SettlementOverrides::amount("50%"));
        response
            .extensions_mut()
            .insert(UptoActualAmount::new("42"));
        let req = PaymentRequirements::new(
            "upto".into(),
            "eip155:8453".parse().unwrap(),
            "1000000".into(),
            "0xpay".into(),
            "0xasset".into(),
            300,
        );
        let amount = resolve_response_settlement_amount(&mut response, &req)
            .unwrap()
            .unwrap();
        assert_eq!(amount, "42");
    }
}
