//! Human-readable currency amount parsing.

use std::fmt;
use std::fmt::Display;
use std::str::FromStr;

use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;

fn strip_non_numeric(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect()
}

/// A non-negative decimal parsed from a human-readable currency string.
///
/// `"10.50"` has [`Self::scale`] 2 and [`Self::mantissa`] 1050.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoneyAmount(pub Decimal);

impl MoneyAmount {
    /// Decimal places in the original input.
    #[must_use]
    pub const fn scale(&self) -> u32 {
        self.0.scale()
    }

    /// Absolute mantissa (no decimal point). `"12.34"` returns `1234`.
    #[must_use]
    pub const fn mantissa(&self) -> u128 {
        self.0.mantissa().unsigned_abs()
    }
}

/// Failure parsing a [`MoneyAmount`].
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[non_exhaustive]
pub enum MoneyAmountParseError {
    /// Input is not a number after stripping currency glyphs.
    #[error("Invalid number format")]
    InvalidFormat,
    /// Value is outside `{MIN_STR}..=MAX_STR`.
    #[error(
        "Amount must be between {} and {}",
        constants::MIN_STR,
        constants::MAX_STR
    )]
    OutOfRange,
    /// Sign is negative.
    #[error("Negative value is not allowed")]
    Negative,
    /// Input has more fractional digits than the token.
    #[error("Too big of a precision: {money} vs {token} on token")]
    WrongPrecision {
        /// Decimal places in the input.
        money: u32,
        /// Decimal places supported by the token.
        token: u32,
    },
}

mod constants {
    use super::Decimal;

    pub(super) const MIN_STR: &str = "0.000000001";
    pub(super) const MAX_STR: &str = "999999999";

    pub(super) const MIN: Decimal = Decimal::from_parts(1, 0, 0, false, 9);
    pub(super) const MAX: Decimal = Decimal::from_parts(999_999_999, 0, 0, false, 0);
}

impl MoneyAmount {
    /// Parses a currency string (`"$10.50"`, `"1,000"`, `"100"`).
    ///
    /// # Errors
    ///
    /// Returns an error when the string is not a number, is negative, or is
    /// outside the allowed range.
    pub fn parse(input: &str) -> Result<Self, MoneyAmountParseError> {
        let cleaned = strip_non_numeric(input);

        let parsed =
            Decimal::from_str(&cleaned).map_err(|_| MoneyAmountParseError::InvalidFormat)?;

        if parsed.is_sign_negative() {
            return Err(MoneyAmountParseError::Negative);
        }

        if parsed < constants::MIN || parsed > constants::MAX {
            return Err(MoneyAmountParseError::OutOfRange);
        }

        Ok(Self(parsed))
    }
}

impl FromStr for MoneyAmount {
    type Err = MoneyAmountParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<&str> for MoneyAmount {
    type Error = MoneyAmountParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

impl From<u128> for MoneyAmount {
    fn from(value: u128) -> Self {
        Self(Decimal::from(value))
    }
}

impl TryFrom<f64> for MoneyAmount {
    type Error = MoneyAmountParseError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        let decimal = Decimal::from_f64(value).ok_or(MoneyAmountParseError::OutOfRange)?;
        if decimal.is_sign_negative() {
            return Err(MoneyAmountParseError::Negative);
        }
        if decimal < constants::MIN || decimal > constants::MAX {
            return Err(MoneyAmountParseError::OutOfRange);
        }
        Ok(Self(decimal))
    }
}

impl Display for MoneyAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.normalize())
    }
}

/// Integer type that can be built as `mantissa × 10^scale_diff`.
pub trait ScaleFromMantissa: Sized {
    /// Constructs `mantissa × 10^scale_diff`.
    ///
    /// # Errors
    ///
    /// Returns [`MoneyAmountParseError::OutOfRange`] on overflow.
    fn from_mantissa_scaled(mantissa: u128, scale_diff: u32)
    -> Result<Self, MoneyAmountParseError>;
}

impl ScaleFromMantissa for u64 {
    fn from_mantissa_scaled(
        mantissa: u128,
        scale_diff: u32,
    ) -> Result<Self, MoneyAmountParseError> {
        let multiplier = 10u64
            .checked_pow(scale_diff)
            .ok_or(MoneyAmountParseError::OutOfRange)?;
        let digits = Self::try_from(mantissa).map_err(|_| MoneyAmountParseError::OutOfRange)?;
        digits
            .checked_mul(multiplier)
            .ok_or(MoneyAmountParseError::OutOfRange)
    }
}

impl ScaleFromMantissa for u128 {
    fn from_mantissa_scaled(
        mantissa: u128,
        scale_diff: u32,
    ) -> Result<Self, MoneyAmountParseError> {
        let multiplier = 10u128
            .checked_pow(scale_diff)
            .ok_or(MoneyAmountParseError::OutOfRange)?;
        mantissa
            .checked_mul(multiplier)
            .ok_or(MoneyAmountParseError::OutOfRange)
    }
}

impl MoneyAmount {
    /// Scales this amount into token units at `decimals` places.
    ///
    /// # Errors
    ///
    /// Returns an error when input precision exceeds `decimals` or the scaled
    /// value overflows `T`.
    pub fn to_token_amount<T: ScaleFromMantissa>(
        self,
        decimals: u8,
    ) -> Result<T, MoneyAmountParseError> {
        let scale = self.scale();
        let token_scale = u32::from(decimals);
        if scale > token_scale {
            return Err(MoneyAmountParseError::WrongPrecision {
                money: scale,
                token: token_scale,
            });
        }
        let scale_diff = token_scale - scale;
        T::from_mantissa_scaled(self.mantissa(), scale_diff)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "unit tests panic on assertion failure"
)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_currency_strings() {
        assert_eq!(MoneyAmount::parse("100").unwrap().to_string(), "100");
        let money = MoneyAmount::parse("$10.50").unwrap();
        assert_eq!(money.scale(), 2);
        assert_eq!(money.mantissa(), 1050);
        assert_eq!(MoneyAmount::parse("1,000").unwrap().mantissa(), 1000);
    }

    #[test]
    fn rejects_negative_and_unparseable() {
        assert!(matches!(
            MoneyAmount::parse("-1"),
            Err(MoneyAmountParseError::Negative)
        ));
        assert!(matches!(
            MoneyAmount::parse("abc"),
            Err(MoneyAmountParseError::InvalidFormat)
        ));
    }

    #[test]
    fn scales_to_token_amount() {
        let amount = MoneyAmount::parse("1.50").unwrap();
        let units: u64 = amount.to_token_amount(6).unwrap();
        assert_eq!(units, 1_500_000);
    }

    #[test]
    fn rejects_excess_precision() {
        let amount = MoneyAmount::parse("1.234").unwrap();
        let err = amount.to_token_amount::<u64>(2).unwrap_err();
        assert!(matches!(
            err,
            MoneyAmountParseError::WrongPrecision { money: 3, token: 2 }
        ));
    }
}
