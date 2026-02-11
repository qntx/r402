//! Unix timestamp utilities for x402 payment authorization windows.
//!
//! This module provides the [`UnixTimestamp`] type used throughout the x402 protocol
//! to represent time-bounded payment authorizations. Timestamps are used in ERC-3009
//! `transferWithAuthorization` messages and Solana payment instructions to specify
//! when a payment authorization becomes valid and when it expires.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::{Display, Formatter};
use std::ops::Add;
use std::time::SystemTime;

/// A Unix timestamp representing seconds since the Unix epoch (1970-01-01T00:00:00Z).
///
/// This type is used throughout the x402 protocol for time-bounded payment authorizations:
///
/// - **`validAfter`**: The earliest time a payment authorization can be executed
/// - **`validBefore`**: The latest time a payment authorization remains valid
///
/// # Serialization
///
/// Serialized as a stringified integer to avoid loss of precision in JSON, since
/// `JavaScript`'s `Number` type cannot safely represent all 64-bit integers.
///
/// ```json
/// "1699999999"
/// ```
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Ord, Eq)]
pub struct UnixTimestamp(u64);

impl Serialize for UnixTimestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for UnixTimestamp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let ts = s
            .parse::<u64>()
            .map_err(|_| serde::de::Error::custom("timestamp must be a non-negative integer"))?;
        Ok(Self(ts))
    }
}

impl Display for UnixTimestamp {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Add<u64> for UnixTimestamp {
    type Output = Self;

    fn add(self, rhs: u64) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl UnixTimestamp {
    /// Creates a new [`UnixTimestamp`] from a raw seconds value.
    #[must_use]
    pub const fn from_secs(secs: u64) -> Self {
        Self(secs)
    }

    /// Returns the current system time as a [`UnixTimestamp`].
    ///
    /// # Panics
    ///
    /// Panics if the system clock is set to a time before the Unix epoch,
    /// which should never happen on properly configured systems.
    #[must_use]
    pub fn now() -> Self {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("SystemTime before UNIX epoch?!?")
            .as_secs();
        Self(now)
    }

    /// Returns the timestamp as raw seconds since the Unix epoch.
    #[must_use]
    pub const fn as_secs(&self) -> u64 {
        self.0
    }
}
