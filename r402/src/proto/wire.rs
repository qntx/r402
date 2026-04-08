//! Wire format utility types for x402 protocol serialization.
//!
//! Standalone encoding and precision-preserving types that appear across
//! multiple protocol messages.

use std::collections::HashMap;
use std::fmt::{self, Display, Formatter};
use std::ops::Add;
use std::str::FromStr;
use std::time::SystemTime;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as b64;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_with::serde_as;

/// A wrapper for base64-encoded byte data.
///
/// This type holds bytes that represent base64-encoded data and provides
/// methods for encoding and decoding.
///
/// # Examples
///
/// ```
/// use r402::proto::Base64Bytes;
///
/// let encoded = Base64Bytes::encode(b"hello world");
/// let decoded = encoded.decode().unwrap();
/// assert_eq!(decoded, b"hello world");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base64Bytes(pub Vec<u8>);

impl Base64Bytes {
    /// Decodes the base64 string bytes to raw binary data.
    ///
    /// # Errors
    ///
    /// Returns an error if the data is not valid base64.
    pub fn decode(&self) -> Result<Vec<u8>, base64::DecodeError> {
        b64.decode(&self.0)
    }

    /// Encodes raw binary data into base64 string bytes.
    pub fn encode<T: AsRef<[u8]>>(input: T) -> Self {
        let encoded = b64.encode(input.as_ref());
        Self(encoded.into_bytes())
    }
}

impl AsRef<[u8]> for Base64Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<&[u8]> for Base64Bytes {
    fn from(slice: &[u8]) -> Self {
        Self(slice.to_vec())
    }
}

impl Display for Base64Bytes {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", String::from_utf8_lossy(&self.0))
    }
}

/// A Unix timestamp representing seconds since the Unix epoch (1970-01-01T00:00:00Z).
///
/// Used throughout the x402 protocol for time-bounded payment authorizations
/// (`validAfter` / `validBefore`).
///
/// Serialized as a stringified integer to avoid loss of precision in JSON.
///
/// # Examples
///
/// ```
/// use r402::proto::UnixTimestamp;
///
/// let ts = UnixTimestamp::from_secs(1_700_000_000);
/// assert_eq!(ts.as_secs(), 1_700_000_000);
///
/// let later = ts + 3600;
/// assert_eq!(later.as_secs(), 1_700_003_600);
///
/// let json = serde_json::to_string(&ts).unwrap();
/// assert_eq!(json, r#""1700000000""#);
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
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
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
    /// Falls back to epoch (0) if the system clock is before the Unix epoch.
    #[must_use]
    pub fn now() -> Self {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self(now)
    }

    /// Returns the timestamp as raw seconds since the Unix epoch.
    #[must_use]
    pub const fn as_secs(self) -> u64 {
        self.0
    }
}

/// A protocol version marker parameterized by its numeric value.
///
/// Serializes as a bare integer (e.g., `2`) and rejects any other
/// value on deserialization, providing compile-time version safety.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub struct Version<const N: u8>;

impl<const N: u8> Version<N> {
    /// The numeric value of this protocol version.
    pub const VALUE: u8 = N;
}

impl<const N: u8> PartialEq<u8> for Version<N> {
    fn eq(&self, other: &u8) -> bool {
        *other == N
    }
}

impl<const N: u8> From<Version<N>> for u8 {
    fn from(_: Version<N>) -> Self {
        N
    }
}

impl<const N: u8> Display for Version<N> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{N}")
    }
}

impl<const N: u8> Serialize for Version<N> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(N)
    }
}

impl<'de, const N: u8> Deserialize<'de> for Version<N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let v = u8::deserialize(deserializer)?;
        if v == N {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom(format!(
                "expected version {N}, got {v}"
            )))
        }
    }
}

/// Protocol extension data attached to various x402 wire types.
///
/// Keys are extension names; values are arbitrary JSON data specific to each extension.
pub type Extensions = HashMap<String, serde_json::Value>;

/// A `u64` value that serializes as a string.
///
/// Some JSON parsers (particularly in `JavaScript`) cannot accurately represent
/// large integers. This type serializes `u64` values as strings to preserve
/// precision across all platforms.
///
/// # Examples
///
/// ```
/// use r402::proto::U64String;
///
/// let v = U64String::from(42u64);
/// assert_eq!(v.inner(), 42);
///
/// let json = serde_json::to_string(&v).unwrap();
/// assert_eq!(json, r#""42""#);
///
/// let parsed: U64String = serde_json::from_str(r#""42""#).unwrap();
/// assert_eq!(parsed.inner(), 42);
/// ```
#[serde_as]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct U64String(#[serde_as(as = "serde_with::DisplayFromStr")] u64);

impl U64String {
    /// Returns the inner `u64` value.
    #[must_use]
    pub const fn inner(self) -> u64 {
        self.0
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
