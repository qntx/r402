//! JSON-safe primitives used by x402 wire types.
//!
//! These are encoding helpers, not protocol nouns. Domain envelopes
//! (`PaymentRequired`, `VerifyRequest`, …) are siblings in [`crate::wire`].

use std::fmt::{self, Display, Formatter};
use std::ops::Add;
use std::str::FromStr;
use std::time::SystemTime;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_with::{DisplayFromStr, serde_as};

/// A compile-time protocol-version marker that serializes as a bare integer.
///
/// The const parameter `N` encodes the version. Deserialization rejects
/// values other than `N`, giving compile-time and runtime certainty that
/// a message carries the expected protocol version.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub struct Version<const N: u8>;

impl<const N: u8> Version<N> {
    /// The numeric value of this version marker.
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
                "expected protocol version {N}, got {v}"
            )))
        }
    }
}

/// Type alias for the x402 v2 version marker.
pub type Version2 = Version<2>;

/// Singleton value for [`Version2`].
pub const V2: Version2 = Version;

/// Raw bytes holding the base64-encoded ASCII representation of some payload.
///
/// Useful as a field type for x402 wire messages that transport arbitrary
/// binary data (for example, the raw Solana transaction wrapped in
/// [`PaymentPayload`](super::PaymentPayload)). Encoding is eager, decoding
/// is deferred.
///
/// # Examples
///
/// ```
/// use r402_core::wire::Base64Bytes;
///
/// let encoded = Base64Bytes::encode(b"hello world");
/// let decoded = encoded.decode().unwrap();
/// assert_eq!(decoded, b"hello world");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base64Bytes(pub Vec<u8>);

impl Base64Bytes {
    /// Decodes the inner base64 bytes into their raw binary form.
    ///
    /// # Errors
    ///
    /// Returns a [`base64::DecodeError`] when the stored bytes are not
    /// valid base64.
    pub fn decode(&self) -> Result<Vec<u8>, base64::DecodeError> {
        B64.decode(&self.0)
    }

    /// Encodes arbitrary bytes into a [`Base64Bytes`] wrapper.
    #[must_use]
    pub fn encode<T: AsRef<[u8]>>(input: T) -> Self {
        Self(B64.encode(input.as_ref()).into_bytes())
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
        f.write_str(&String::from_utf8_lossy(&self.0))
    }
}

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
