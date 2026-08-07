//! Stringified `u64` for JSON-safe integer transport.

use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use serde_with::{DisplayFromStr, serde_as};

/// A `u64` that serializes as a JSON string.
///
/// JSON integer parsing in the browser (JavaScript `Number`) is limited
/// to 2⁵³ bits. x402 wraps `u64`-sized fields in strings to guarantee
/// exact precision across runtimes.
///
/// # Examples
///
/// ```
/// use r402_core::wire::U64String;
///
/// let value = U64String::from(42_u64);
/// assert_eq!(value.inner(), 42);
/// assert_eq!(serde_json::to_string(&value).unwrap(), r#""42""#);
/// let parsed: U64String = serde_json::from_str(r#""42""#).unwrap();
/// assert_eq!(parsed.inner(), 42);
/// ```
#[serde_as]
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[repr(transparent)]
pub struct U64String(#[serde_as(as = "DisplayFromStr")] u64);

impl U64String {
    /// Returns the wrapped `u64`.
    #[must_use]
    pub const fn inner(self) -> u64 {
        self.0
    }
}

impl Display for U64String {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl FromStr for U64String {
    type Err = <u64 as FromStr>::Err;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u64>().map(Self)
    }
}

impl From<u64> for U64String {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<U64String> for u64 {
    fn from(value: U64String) -> Self {
        value.0
    }
}
