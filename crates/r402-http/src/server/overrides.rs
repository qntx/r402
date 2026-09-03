//! Sequential-only settlement amount overrides (`Settlement-Overrides` / `UptoActualAmount`).

use compact_str::CompactString;
use http::{HeaderMap, HeaderName, HeaderValue};
use r402_protocol::payment::{
    PaymentRequirements, asset_decimals_from_extra, resolve_settlement_override_amount,
};
pub use r402_protocol::payment::{SettlementOverrideError, SettlementOverrides};

/// Official HTTP header for partial settlement.
pub const SETTLEMENT_OVERRIDES_HEADER: &str = "Settlement-Overrides";

/// Lowercase wire name.
#[must_use]
pub const fn settlement_overrides_header_name() -> HeaderName {
    HeaderName::from_static("settlement-overrides")
}

/// Writes `Settlement-Overrides` JSON onto `headers`.
pub fn set_settlement_overrides(headers: &mut HeaderMap, overrides: &SettlementOverrides) {
    if let Ok(json) = serde_json::to_string(overrides)
        && let Ok(value) = HeaderValue::from_str(&json)
    {
        let _ = headers.insert(settlement_overrides_header_name(), value);
    }
}

/// Convenience when the handler holds a full response.
pub fn set_settlement_overrides_on_response(
    response: &mut axum_core::response::Response,
    overrides: &SettlementOverrides,
) {
    set_settlement_overrides(response.headers_mut(), overrides);
}

/// Parses and **removes** `Settlement-Overrides` so it never reaches the client.
#[must_use]
pub fn take_settlement_overrides_header(headers: &mut HeaderMap) -> Option<SettlementOverrides> {
    let value = headers.remove(settlement_overrides_header_name())?;
    let s = value.to_str().ok()?;
    serde_json::from_str(s).ok()
}

/// Resolves handler-declared actual amount (atomic units).
///
/// Precedence: [`UptoActualAmount`] extension, then `Settlement-Overrides`.
///
/// # Errors
///
/// [`SettlementOverrideError`] when the header amount cannot be resolved.
pub fn resolve_response_settlement_amount(
    response: &mut axum_core::response::Response,
    requirements: &PaymentRequirements,
) -> Result<Option<String>, SettlementOverrideError> {
    let header = take_settlement_overrides_header(response.headers_mut());
    if let Some(ext) = response.extensions_mut().remove::<UptoActualAmount>() {
        return Ok(Some(ext.into_inner().into()));
    }
    let Some(overrides) = header else {
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

/// In-process atomic-amount override (already resolved units).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UptoActualAmount(CompactString);

impl UptoActualAmount {
    /// Wraps an atomic decimal string.
    pub fn new<S: Into<CompactString>>(amount: S) -> Self {
        Self(amount.into())
    }

    /// Borrowed atomic amount.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Consumes the wrapper.
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
    use r402_protocol::payment::PaymentRequirements;

    use super::*;

    #[test]
    fn header_roundtrip_and_strip() {
        let mut headers = HeaderMap::new();
        set_settlement_overrides(&mut headers, &SettlementOverrides::amount("50%"));
        assert!(
            headers.contains_key(settlement_overrides_header_name()),
            "header must be present after set"
        );
        let parsed = take_settlement_overrides_header(&mut headers)
            .expect("well-formed Settlement-Overrides must parse");
        assert_eq!(parsed.amount.as_deref(), Some("50%"));
        assert!(
            !headers.contains_key(settlement_overrides_header_name()),
            "take must strip the billing header"
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
            "eip155:8453".parse().expect("fixture chain id"),
            "1000000".into(),
            "0xpay".into(),
            "0xasset".into(),
            300,
        );
        let amount = resolve_response_settlement_amount(&mut response, &req)
            .expect("override resolve")
            .expect("amount present");
        assert_eq!(amount, "42");
        assert!(
            !response
                .headers()
                .contains_key(settlement_overrides_header_name()),
            "Settlement-Overrides must be stripped even when the extension wins"
        );
    }
}
