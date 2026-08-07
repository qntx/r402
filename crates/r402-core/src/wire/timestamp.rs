//! Stringified Unix timestamp.

use std::fmt::{self, Display, Formatter};
use std::ops::Add;
use std::time::SystemTime;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A Unix timestamp in seconds since the epoch (1970-01-01T00:00:00Z).
///
/// The x402 wire format stringifies integer timestamps to guarantee
/// cross-language precision (JavaScript's `Number` type cannot precisely
/// represent all `u64` values). [`UnixTimestamp`] serializes as a JSON
/// string and deserializes from either a JSON string or number.
///
/// # Examples
///
/// ```
/// use r402_core::wire::UnixTimestamp;
///
/// let ts = UnixTimestamp::from_secs(1_700_000_000);
/// assert_eq!(ts.as_secs(), 1_700_000_000);
/// assert_eq!((ts + 3600).as_secs(), 1_700_003_600);
/// assert_eq!(serde_json::to_string(&ts).unwrap(), r#""1700000000""#);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Ord, Eq, Hash)]
pub struct UnixTimestamp(u64);

impl UnixTimestamp {
    /// Constructs a timestamp from raw Unix seconds.
    #[must_use]
    pub const fn from_secs(secs: u64) -> Self {
        Self(secs)
    }

    /// Returns the current system clock as a [`UnixTimestamp`].
    ///
    /// Falls back to the epoch if the clock is before 1970.
    #[must_use]
    pub fn now() -> Self {
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        Self(secs)
    }

    /// Returns the timestamp as raw Unix seconds.
    #[must_use]
    pub const fn as_secs(self) -> u64 {
        self.0
    }
}

impl Display for UnixTimestamp {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl Add<u64> for UnixTimestamp {
    type Output = Self;
    fn add(self, rhs: u64) -> Self::Output {
        Self(self.0.saturating_add(rhs))
    }
}

impl Serialize for UnixTimestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for UnixTimestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = UnixTimestamp;

            fn expecting(&self, f: &mut Formatter<'_>) -> fmt::Result {
                f.write_str("a non-negative integer or its string representation")
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(UnixTimestamp(v))
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                v.parse::<u64>()
                    .map(UnixTimestamp)
                    .map_err(|_| E::custom("expected non-negative integer"))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}
