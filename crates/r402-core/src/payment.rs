//! Typestate lifecycle for a single x402 payment.
//!
//! A payment moves through three well-defined states:
//!
//! ```text
//! Payment<Unverified> ──verify()──▶ Payment<Verified> ──settle()──▶ Payment<Settled>
//! ```
//!
//! Each transition is a method on the corresponding state; the compiler
//! enforces the order (you cannot call `settle` on an `Unverified` payment).
//! All state types are zero-sized so the `Payment<S>` wrapper compiles to the
//! same memory layout as its payload.
//!
//! [`SettlementOverrides`] resolve upto partial-settle amounts (`"1000"`,
//! `"50%"`, `"$0.05"`) into atomic units.
//!
//! # Example
//!
//! ```no_run
//! use r402_core::payment::Payment;
//! use r402_core::Facilitator;
//! use r402_core::wire::VerifyRequest;
//!
//! # async fn run<F: Facilitator>(facilitator: F, payload: VerifyRequest) {
//! let payment = Payment::new(payload.clone());
//! let (verified, _vr) = payment.verify(&facilitator, payload.clone()).await.unwrap();
//! let (_settled, _sr) = verified.settle(&facilitator, payload.into()).await.unwrap();
//! # }
//! ```

use std::marker::PhantomData;
use std::str::FromStr;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::error::FacilitatorError;
use crate::facilitator::Facilitator;
use crate::wire::{SettleRequest, SettleResponse, VerifyRequest, VerifyResponse};

/// Marker indicating the payment has not yet been verified.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unverified;

/// Marker indicating the payment has passed verification.
#[derive(Debug, Clone, Copy, Default)]
pub struct Verified;

/// Marker indicating the payment has been settled.
#[derive(Debug, Clone, Copy, Default)]
pub struct Settled;

/// A payment moving through the verify → settle lifecycle.
///
/// The type parameter `S` tracks the current state at compile time. The
/// struct is transparent (zero overhead) around the underlying request.
#[derive(Debug, Clone)]
pub struct Payment<S> {
    request: VerifyRequest,
    state: PhantomData<fn() -> S>,
}

impl<S> Payment<S> {
    /// Returns a reference to the wire-level verify request this payment
    /// was constructed from.
    #[must_use]
    pub const fn request(&self) -> &VerifyRequest {
        &self.request
    }
}

impl Payment<Unverified> {
    /// Wraps a raw verify request as an unverified payment.
    #[must_use]
    pub const fn new(request: VerifyRequest) -> Self {
        Self {
            request,
            state: PhantomData,
        }
    }

    /// Runs facilitator verification.
    ///
    /// On success, returns the verified payment together with the
    /// facilitator's [`VerifyResponse`]. On failure, the payment is
    /// consumed and a [`FacilitatorError`] is returned.
    ///
    /// # Errors
    ///
    /// Propagates any [`FacilitatorError`] raised by the underlying
    /// facilitator. A `Valid` verification that returns `VerifyResponse::Invalid`
    /// is surfaced as `Err(FacilitatorError::Verification(VerificationError::InvalidFormat))`.
    pub async fn verify<F: Facilitator>(
        self,
        facilitator: &F,
        request: VerifyRequest,
    ) -> Result<(Payment<Verified>, VerifyResponse), FacilitatorError> {
        let response = facilitator.verify(request).await?;
        match &response {
            VerifyResponse::Valid { .. } => Ok((
                Payment {
                    request: self.request,
                    state: PhantomData,
                },
                response,
            )),
            VerifyResponse::Invalid {
                reason, message, ..
            } => Err(FacilitatorError::Verification(
                crate::error::VerificationError::InvalidFormat(format!(
                    "{reason}: {}",
                    message.as_deref().unwrap_or(""),
                )),
            )),
        }
    }
}

impl Payment<Verified> {
    /// Runs facilitator settlement on a previously verified payment.
    ///
    /// # Errors
    ///
    /// Propagates any [`FacilitatorError`] raised by the facilitator; a
    /// `SettleResponse::Failure` is surfaced as
    /// `Err(FacilitatorError::Onchain(...))`.
    pub async fn settle<F: Facilitator>(
        self,
        facilitator: &F,
        request: SettleRequest,
    ) -> Result<(Payment<Settled>, SettleResponse), FacilitatorError> {
        let response = facilitator.settle(request).await?;
        match &response {
            SettleResponse::Success { .. } => Ok((
                Payment {
                    request: self.request,
                    state: PhantomData,
                },
                response,
            )),
            SettleResponse::Failure { message, .. } => Err(FacilitatorError::Onchain(
                message.as_deref().unwrap_or("").to_owned(),
            )),
        }
    }
}

impl Payment<Settled> {
    /// Extracts the underlying wire-level request.
    #[must_use]
    pub fn into_request(self) -> VerifyRequest {
        self.request
    }
}

#[cfg(test)]
mod typestate_tests {
    use std::future::Future;

    use super::*;
    use crate::error::ErrorReason;
    use crate::wire::{Extensions, SettleResponse, SupportedResponse, VerifyResponse};

    struct AlwaysValid;

    impl Facilitator for AlwaysValid {
        fn verify(
            &self,
            _request: VerifyRequest,
        ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(VerifyResponse::valid("0xPAYER")))
        }

        fn settle(
            &self,
            _request: SettleRequest,
        ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(SettleResponse::Success {
                payer: "0xPAYER".into(),
                transaction: "0xTX".into(),
                network: "eip155:1".into(),
                amount: Some("1000000".into()),
                extensions: Extensions::new(),
            }))
        }

        fn supported(
            &self,
        ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(SupportedResponse::default()))
        }
    }

    struct AlwaysInvalid;

    impl Facilitator for AlwaysInvalid {
        fn verify(
            &self,
            _request: VerifyRequest,
        ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(VerifyResponse::invalid(
                None,
                ErrorReason::InvalidPayload,
            )))
        }

        fn settle(
            &self,
            _request: SettleRequest,
        ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
            std::future::ready(Err(FacilitatorError::Onchain("unreachable".into())))
        }

        fn supported(
            &self,
        ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(SupportedResponse::default()))
        }
    }

    fn dummy_request() -> VerifyRequest {
        serde_json::json!({}).into()
    }

    #[tokio::test]
    async fn verify_then_settle_succeeds() {
        let facilitator = AlwaysValid;
        let payment = Payment::new(dummy_request());
        let (verified, verify_resp) = payment.verify(&facilitator, dummy_request()).await.unwrap();
        assert!(verify_resp.is_valid());
        let (_settled, settle_resp) = verified
            .settle(&facilitator, dummy_request().into())
            .await
            .unwrap();
        assert!(settle_resp.is_success());
    }

    #[tokio::test]
    async fn verify_invalid_propagates() {
        let facilitator = AlwaysInvalid;
        let payment = Payment::new(dummy_request());
        let err = payment
            .verify(&facilitator, dummy_request())
            .await
            .unwrap_err();
        assert!(matches!(err, FacilitatorError::Verification(_)));
    }
}

/// Official wire shape for partial settlement (upto billing by actual usage).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct SettlementOverrides {
    /// Amount to settle. Formats: atomic `"1000"`, percent `"50%"`, dollar `"$0.05"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,
}

impl SettlementOverrides {
    /// Builds overrides with a single amount string.
    #[must_use]
    pub fn amount(amount: impl Into<String>) -> Self {
        Self {
            amount: Some(amount.into()),
        }
    }
}

/// Failure resolving a settlement override amount string.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SettlementOverrideError {
    /// Authorized maximum is not a base-10 integer.
    #[error("invalid requirements amount: {0}")]
    InvalidRequirementsAmount(String),
    /// Percent or dollar override failed to parse / convert.
    #[error("invalid settlement override amount: {0}")]
    InvalidOverride(String),
    /// Intermediate arithmetic overflowed.
    #[error("settlement override arithmetic overflow")]
    Overflow,
}

/// Default ERC-20 style decimals when `extra.decimals` is absent (matches Go).
pub const DEFAULT_ASSET_DECIMALS: u32 = 6;

/// Reads `decimals` from payment-requirements `extra` (number or numeric string).
///
/// Falls back to [`DEFAULT_ASSET_DECIMALS`].
#[must_use]
pub fn asset_decimals_from_extra(extra: Option<&serde_json::Value>) -> u32 {
    let Some(extra) = extra else {
        return DEFAULT_ASSET_DECIMALS;
    };
    match extra.get("decimals") {
        Some(serde_json::Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(DEFAULT_ASSET_DECIMALS),
        Some(serde_json::Value::String(s)) => s.parse().unwrap_or(DEFAULT_ASSET_DECIMALS),
        _ => DEFAULT_ASSET_DECIMALS,
    }
}

/// Resolves a settlement override amount to atomic units (decimal string).
///
/// # Errors
///
/// Returns [`SettlementOverrideError`] when the override or authorized max
/// cannot be parsed, or intermediate arithmetic overflows `u128`.
pub fn resolve_settlement_override_amount(
    raw_amount: &str,
    authorized_max: &str,
    decimals: u32,
) -> Result<String, SettlementOverrideError> {
    let raw = raw_amount.trim();
    if let Some(percent_body) = raw.strip_suffix('%') {
        return resolve_percent(percent_body.trim(), authorized_max);
    }
    if let Some(dollar_body) = raw.strip_prefix('$') {
        return resolve_dollar(dollar_body.trim(), decimals);
    }
    // Raw atomic units: pass through after validating base-10 digits.
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SettlementOverrideError::InvalidOverride(raw.to_owned()));
    }
    Ok(raw.to_owned())
}

fn resolve_percent(percent: &str, authorized_max: &str) -> Result<String, SettlementOverrideError> {
    // Up to 2 decimal places: "50" / "50.5" / "50.25" → scaled hundredths of a percent.
    let (int_part, frac_part) = match percent.split_once('.') {
        Some((i, f)) => (i, f),
        None => (percent, ""),
    };
    if int_part.is_empty()
        || !int_part.bytes().all(|b| b.is_ascii_digit())
        || frac_part.len() > 2
        || !frac_part.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(SettlementOverrideError::InvalidOverride(format!(
            "{percent}%"
        )));
    }
    let int_val: u128 = int_part
        .parse()
        .map_err(|_| SettlementOverrideError::InvalidOverride(format!("{percent}%")))?;
    let frac_padded = format!("{frac_part:0<2}");
    let frac_val: u128 = if frac_padded.is_empty() {
        0
    } else {
        frac_padded
            .parse()
            .map_err(|_| SettlementOverrideError::InvalidOverride(format!("{percent}%")))?
    };
    // scaledPercent = int*100 + frac  (percent with 2 d.p. × 100)
    let scaled_percent = int_val
        .checked_mul(100)
        .and_then(|v| v.checked_add(frac_val))
        .ok_or(SettlementOverrideError::Overflow)?;

    let base = parse_u128_amount(authorized_max)?;
    // result = base * scaledPercent / 10000
    let product = base
        .checked_mul(scaled_percent)
        .ok_or(SettlementOverrideError::Overflow)?;
    Ok((product / 10_000).to_string())
}

fn resolve_dollar(dollar: &str, decimals: u32) -> Result<String, SettlementOverrideError> {
    let dollar_dec = Decimal::from_str(dollar)
        .map_err(|_| SettlementOverrideError::InvalidOverride(format!("${dollar}")))?;
    if dollar_dec.is_sign_negative() {
        return Err(SettlementOverrideError::InvalidOverride(format!(
            "${dollar}"
        )));
    }
    let scale = Decimal::from(10u64.pow(decimals.min(18)));
    let atomic = dollar_dec
        .checked_mul(scale)
        .ok_or(SettlementOverrideError::Overflow)?;
    // Floor toward zero for positive values (matches Go big.Float.Int).
    let truncated = atomic.trunc();
    let s = truncated.normalize().to_string();
    // Decimal may emit "1000.0" — strip fractional part if present after trunc.
    Ok(s.split('.').next().unwrap_or("0").to_owned())
}

fn parse_u128_amount(raw: &str) -> Result<u128, SettlementOverrideError> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SettlementOverrideError::InvalidRequirementsAmount(
            raw.to_owned(),
        ));
    }
    raw.parse()
        .map_err(|_| SettlementOverrideError::InvalidRequirementsAmount(raw.to_owned()))
}

#[cfg(test)]
mod override_tests {
    use super::*;

    #[test]
    fn atomic_passthrough() {
        assert_eq!(
            resolve_settlement_override_amount("500", "1000000", 6).unwrap(),
            "500"
        );
    }

    #[test]
    fn percent_half() {
        assert_eq!(
            resolve_settlement_override_amount("50%", "1000000", 6).unwrap(),
            "500000"
        );
    }

    #[test]
    fn percent_with_fraction() {
        // 12.5% of 1000 = 125
        assert_eq!(
            resolve_settlement_override_amount("12.5%", "1000", 6).unwrap(),
            "125"
        );
    }

    #[test]
    fn dollar_default_decimals() {
        assert_eq!(
            resolve_settlement_override_amount("$0.001", "1000000", 6).unwrap(),
            "1000"
        );
    }

    #[test]
    fn rejects_empty_atomic() {
        assert!(resolve_settlement_override_amount("", "1", 6).is_err());
    }

    #[test]
    fn decimals_from_extra() {
        let v = serde_json::json!({"decimals": 18});
        assert_eq!(asset_decimals_from_extra(Some(&v)), 18);
        assert_eq!(asset_decimals_from_extra(None), 6);
    }
}
