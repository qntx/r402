//! Exact integer arithmetic for CSPR motes.
//!
//! CSPR has **9 decimals**; its smallest indivisible unit is the *mote*
//! (1 CSPR = 1 000 000 000 motes). wCSPR — the CEP-18 wrapper used for x402
//! settlement — inherits the same precision.
//!
//! Every conversion in this module is performed with integer arithmetic on
//! the decimal digits of the input. A value carrying more than nine
//! fractional digits is **rejected** with
//! [`MotesParseError::SubMotePrecision`]: silently truncating a buyer's
//! authorisation would make the signed amount and the settled amount
//! disagree, which the facilitator would then reject on-chain.

use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Number of decimal places of CSPR / wCSPR.
pub const CSPR_DECIMALS: u8 = 9;

/// Number of motes in one whole CSPR (`10^9`).
pub const MOTES_PER_CSPR: u128 = 1_000_000_000;

/// Errors produced while converting a decimal CSPR string into motes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MotesParseError {
    /// The input was empty or contained no digits.
    #[error("amount is empty")]
    Empty,
    /// The input contained a character that is not a digit or a single
    /// decimal point.
    #[error("invalid character {0:?} in amount")]
    InvalidCharacter(char),
    /// The input contained more than one decimal point.
    #[error("amount contains more than one decimal point")]
    MultipleDecimalPoints,
    /// The fractional part carried more precision than a mote can express.
    ///
    /// Truncation is never performed: an amount that cannot be represented
    /// exactly is an error the caller must handle.
    #[error("amount has {digits} fractional digits, CSPR supports at most {CSPR_DECIMALS}")]
    SubMotePrecision {
        /// Number of fractional digits supplied.
        digits: usize,
    },
    /// The value does not fit in a `u128` mote count.
    #[error("amount overflows the mote representation")]
    Overflow,
}

/// A CSPR / wCSPR amount denominated in motes.
///
/// Serialises as a **decimal string** to match the x402 v2 wire format,
/// where every amount field is a base-unit integer encoded as a string.
///
/// # Examples
///
/// ```
/// use r402_casper::Motes;
///
/// let motes: Motes = "1.5".parse::<Motes>().unwrap();
/// assert_eq!(motes.inner(), 1_500_000_000);
/// assert_eq!(motes.to_cspr_string(), "1.5");
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Motes(u128);

impl Motes {
    /// Zero motes.
    pub const ZERO: Self = Self(0);

    /// Wraps a raw mote count.
    #[must_use]
    pub const fn new(motes: u128) -> Self {
        Self(motes)
    }

    /// Returns the raw mote count.
    #[must_use]
    pub const fn inner(self) -> u128 {
        self.0
    }

    /// Returns `true` when the amount is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Converts a whole-CSPR count into motes.
    ///
    /// # Errors
    ///
    /// Returns [`MotesParseError::Overflow`] when the product exceeds `u128`.
    pub const fn from_cspr(cspr: u128) -> Result<Self, MotesParseError> {
        match cspr.checked_mul(MOTES_PER_CSPR) {
            Some(motes) => Ok(Self(motes)),
            None => Err(MotesParseError::Overflow),
        }
    }

    /// Parses a decimal CSPR amount (e.g. `"0.000000001"`) into motes.
    ///
    /// Leading/trailing ASCII whitespace and a single leading `+` are
    /// tolerated; everything else must be digits and at most one `.`.
    ///
    /// # Errors
    ///
    /// Returns [`MotesParseError`] when the input is malformed, carries
    /// sub-mote precision, or overflows `u128`.
    pub fn from_cspr_str(input: &str) -> Result<Self, MotesParseError> {
        let trimmed = input.trim().strip_prefix('+').unwrap_or(input.trim());
        if trimmed.is_empty() {
            return Err(MotesParseError::Empty);
        }

        let (integer, fraction) = match trimmed.split_once('.') {
            Some((_, rest)) if rest.contains('.') => {
                return Err(MotesParseError::MultipleDecimalPoints);
            }
            Some((head, rest)) => (head, rest),
            None => (trimmed, ""),
        };

        if integer.is_empty() && fraction.is_empty() {
            return Err(MotesParseError::Empty);
        }
        if let Some(bad) = integer
            .chars()
            .chain(fraction.chars())
            .find(|c| !c.is_ascii_digit())
        {
            return Err(MotesParseError::InvalidCharacter(bad));
        }

        let decimals = usize::from(CSPR_DECIMALS);
        if fraction.len() > decimals {
            // Reject rather than truncate: see the module-level rationale.
            return Err(MotesParseError::SubMotePrecision {
                digits: fraction.len(),
            });
        }

        let whole = parse_u128(integer)?;
        let scaled = whole
            .checked_mul(MOTES_PER_CSPR)
            .ok_or(MotesParseError::Overflow)?;

        let frac_value = parse_u128(fraction)?;
        let padding = decimals - fraction.len();
        let multiplier = 10u128
            .checked_pow(u32::try_from(padding).map_err(|_| MotesParseError::Overflow)?)
            .ok_or(MotesParseError::Overflow)?;
        let frac_motes = frac_value
            .checked_mul(multiplier)
            .ok_or(MotesParseError::Overflow)?;

        scaled
            .checked_add(frac_motes)
            .map(Self)
            .ok_or(MotesParseError::Overflow)
    }

    /// Renders the amount as a decimal CSPR string with trailing fractional
    /// zeros removed (`1_500_000_000` motes ⇒ `"1.5"`).
    #[must_use]
    pub fn to_cspr_string(self) -> String {
        let whole = self.0 / MOTES_PER_CSPR;
        let fraction = self.0 % MOTES_PER_CSPR;
        if fraction == 0 {
            return whole.to_string();
        }
        let decimals = usize::from(CSPR_DECIMALS);
        let padded = format!("{fraction:0decimals$}");
        let trimmed = padded.trim_end_matches('0');
        format!("{whole}.{trimmed}")
    }

    /// Checked addition of two mote amounts.
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(sum) => Some(Self(sum)),
            None => None,
        }
    }

    /// Checked subtraction of two mote amounts.
    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        match self.0.checked_sub(other.0) {
            Some(diff) => Some(Self(diff)),
            None => None,
        }
    }
}

fn parse_u128(digits: &str) -> Result<u128, MotesParseError> {
    if digits.is_empty() {
        return Ok(0);
    }
    digits.parse::<u128>().map_err(|_| MotesParseError::Overflow)
}

impl Display for Motes {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl FromStr for Motes {
    type Err = MotesParseError;

    /// Parses a **decimal CSPR** amount. Use [`Motes::new`] for raw mote
    /// counts already expressed in base units.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_cspr_str(s)
    }
}

impl From<u128> for Motes {
    fn from(value: u128) -> Self {
        Self(value)
    }
}

impl From<u64> for Motes {
    fn from(value: u64) -> Self {
        Self(u128::from(value))
    }
}

impl From<Motes> for u128 {
    fn from(value: Motes) -> Self {
        value.0
    }
}

impl Serialize for Motes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for Motes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(serde::de::Error::custom(MotesParseError::Empty));
        }
        if let Some(bad) = raw.chars().find(|c| !c.is_ascii_digit()) {
            return Err(serde::de::Error::custom(
                MotesParseError::InvalidCharacter(bad),
            ));
        }
        raw.parse::<u128>()
            .map(Self)
            .map_err(|_| serde::de::Error::custom(MotesParseError::Overflow))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_whole_cspr() {
        assert_eq!(Motes::from_cspr_str("1").unwrap().inner(), 1_000_000_000);
        assert_eq!(Motes::from_cspr_str("0").unwrap().inner(), 0);
        assert_eq!(
            Motes::from_cspr_str("1234").unwrap().inner(),
            1_234_000_000_000
        );
    }

    #[test]
    fn parses_fractional_cspr_exactly() {
        assert_eq!(Motes::from_cspr_str("1.5").unwrap().inner(), 1_500_000_000);
        assert_eq!(Motes::from_cspr_str("0.1").unwrap().inner(), 100_000_000);
        assert_eq!(
            Motes::from_cspr_str("12.000000001").unwrap().inner(),
            12_000_000_001
        );
    }

    #[test]
    fn parses_one_mote() {
        assert_eq!(Motes::from_cspr_str("0.000000001").unwrap().inner(), 1);
    }

    #[test]
    fn rejects_sub_mote_precision_instead_of_truncating() {
        let err = Motes::from_cspr_str("0.0000000001").unwrap_err();
        assert_eq!(err, MotesParseError::SubMotePrecision { digits: 10 });
    }

    #[test]
    fn rejects_sub_mote_precision_even_when_trailing_digits_are_zero() {
        // A trailing zero still exceeds the declared precision; accepting it
        // would make the wire amount and the signed amount disagree.
        let err = Motes::from_cspr_str("1.0000000000").unwrap_err();
        assert_eq!(err, MotesParseError::SubMotePrecision { digits: 10 });
    }

    #[test]
    fn rejects_multiple_decimal_points() {
        assert_eq!(
            Motes::from_cspr_str("1.2.3").unwrap_err(),
            MotesParseError::MultipleDecimalPoints
        );
    }

    #[test]
    fn rejects_non_numeric_input() {
        assert_eq!(
            Motes::from_cspr_str("1a").unwrap_err(),
            MotesParseError::InvalidCharacter('a')
        );
        assert_eq!(
            Motes::from_cspr_str("-1").unwrap_err(),
            MotesParseError::InvalidCharacter('-')
        );
        assert_eq!(Motes::from_cspr_str("   ").unwrap_err(), MotesParseError::Empty);
    }

    #[test]
    fn accepts_bare_fraction_and_bare_integer_forms() {
        assert_eq!(Motes::from_cspr_str(".5").unwrap().inner(), 500_000_000);
        assert_eq!(Motes::from_cspr_str("5.").unwrap().inner(), 5_000_000_000);
    }

    #[test]
    fn rejects_overflowing_amounts() {
        let huge = "1".repeat(40);
        assert_eq!(
            Motes::from_cspr_str(&huge).unwrap_err(),
            MotesParseError::Overflow
        );
    }

    #[test]
    fn renders_decimal_cspr() {
        assert_eq!(Motes::new(1_500_000_000).to_cspr_string(), "1.5");
        assert_eq!(Motes::new(1).to_cspr_string(), "0.000000001");
        assert_eq!(Motes::new(2_000_000_000).to_cspr_string(), "2");
        assert_eq!(Motes::ZERO.to_cspr_string(), "0");
    }

    #[test]
    fn cspr_string_round_trips() {
        for raw in [0u128, 1, 999_999_999, 1_000_000_000, 123_456_789_987_654_321] {
            let motes = Motes::new(raw);
            let rendered = motes.to_cspr_string();
            assert_eq!(
                Motes::from_cspr_str(&rendered).unwrap(),
                motes,
                "round trip failed for {raw}"
            );
        }
    }

    #[test]
    fn from_cspr_multiplies_exactly() {
        assert_eq!(Motes::from_cspr(3).unwrap().inner(), 3_000_000_000);
        assert_eq!(Motes::from_cspr(u128::MAX), Err(MotesParseError::Overflow));
    }

    #[test]
    fn serialises_as_base_unit_string() {
        let motes = Motes::new(1_500_000_000);
        assert_eq!(
            serde_json::to_string(&motes).unwrap(),
            r#""1500000000""#,
            "amounts must be base-unit strings on the wire"
        );
        let decoded: Motes = serde_json::from_str(r#""1500000000""#).unwrap();
        assert_eq!(decoded, motes);
    }

    #[test]
    fn deserialisation_rejects_decimal_wire_amounts() {
        // The wire carries base units only — a decimal point here means the
        // producer forgot to scale, which must not be silently accepted.
        assert!(serde_json::from_str::<Motes>(r#""1.5""#).is_err());
        assert!(serde_json::from_str::<Motes>(r#""""#).is_err());
    }

    #[test]
    fn checked_arithmetic_saturates_to_none() {
        assert_eq!(
            Motes::new(2).checked_add(Motes::new(3)),
            Some(Motes::new(5))
        );
        assert_eq!(Motes::new(2).checked_sub(Motes::new(3)), None);
        assert_eq!(Motes::new(u128::MAX).checked_add(Motes::new(1)), None);
    }
}
