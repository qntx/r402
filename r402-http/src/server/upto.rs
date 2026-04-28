//! HTTP-level support for the x402 "upto" scheme.
//!
//! The upto scheme lets a buyer authorise a **maximum** payment and lets the
//! resource server decide the actual charge at request time (e.g. for
//! usage-based pricing). This module defines the
//! [`UptoActualAmount`] response extension that handlers use to communicate
//! the final charge back to the r402 middleware.
//!
//! # Flow
//!
//! ```text
//! client          middleware           handler
//!   |                 |                    |
//!   | POST + sig ---->| verify             |
//!   |                 |  ok                |
//!   |                 |---- request ------>|
//!   |                 |                    |  (compute usage,
//!   |                 |                    |   set extension)
//!   |                 |<--- response + ext |
//!   |                 | read UptoActualAmount
//!   |                 |    → override
//!   |                 |    → settle(actual)
//!   |<------- resp ---|
//! ```
//!
//! # Example
//!
//! ```ignore
//! use axum::response::IntoResponse;
//! use r402_http::server::UptoActualAmount;
//!
//! async fn handler() -> impl IntoResponse {
//!     let mut response = "Hello".into_response();
//!     response
//!         .extensions_mut()
//!         .insert(UptoActualAmount::new("125000")); // 0.125 USDC
//!     response
//! }
//! ```
//!
//! # Compatibility
//!
//! Only [`SettlementMode::Sequential`](super::SettlementMode::Sequential)
//! honours this extension: concurrent and background modes spawn settlement
//! before the handler returns, so the override has nowhere to land. Mixing
//! upto with those modes silently charges the signed maximum.

use compact_str::CompactString;

/// Response extension instructing the r402 middleware to settle the upto
/// payment for this specific amount (base units as a decimal string).
///
/// Inserted by application handlers into [`Response::extensions_mut`] so the
/// middleware can patch `paymentRequirements.amount` before forwarding the
/// settle request to the facilitator.
///
/// The value MUST be less than or equal to the authorised maximum from the
/// buyer's signed payload; otherwise the facilitator returns
/// [`ErrorReason::InvalidUptoEvmPayloadSettlementExceedsAmount`](r402_core::error_reason::ErrorReason::InvalidUptoEvmPayloadSettlementExceedsAmount).
///
/// [`Response::extensions_mut`]: axum_core::response::Response::extensions_mut
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UptoActualAmount(CompactString);

impl UptoActualAmount {
    /// Creates a new override from a decimal-string amount.
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

#[cfg(test)]
mod tests {
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
}
