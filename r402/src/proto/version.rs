//! Protocol version marker type.
//!
//! Provides [`Version<N>`], a const-generic version marker that serializes
//! as a bare integer and rejects mismatched values on deserialization.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A protocol version marker parameterized by its numeric value.
///
/// Serializes as a bare integer (e.g., `1` or `2`) and rejects any other
/// value on deserialization, providing compile-time version safety.
///
/// Use the type alias [`super::v2::X402Version2`] instead of constructing this directly.
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

impl<const N: u8> std::fmt::Display for Version<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
