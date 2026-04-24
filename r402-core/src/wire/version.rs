//! Const-parameterized protocol version marker.

use std::fmt::{self, Display, Formatter};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

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
