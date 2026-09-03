//! CAIP-2 `namespace:reference` chain identifier.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// CAIP-2 blockchain identifier (`eip155:8453`, `solana:…`).
///
/// Serializes as a colon-separated string.
///
/// # Examples
///
/// ```
/// use r402_protocol::ChainId;
///
/// let chain = ChainId::new("eip155", "8453");
/// assert_eq!(chain.namespace(), "eip155");
/// assert_eq!(chain.reference(), "8453");
/// assert_eq!(chain.to_string(), "eip155:8453");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChainId {
    namespace: String,
    reference: String,
}

impl ChainId {
    /// Creates a chain ID from namespace and reference.
    pub fn new<N: Into<String>, R: Into<String>>(namespace: N, reference: R) -> Self {
        Self {
            namespace: namespace.into(),
            reference: reference.into(),
        }
    }

    /// Namespace component (`eip155`, `solana`, …).
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Reference component (`8453`, genesis hash, …).
    #[must_use]
    pub fn reference(&self) -> &str {
        &self.reference
    }

    /// Consumes the chain ID and returns `(namespace, reference)`.
    #[must_use]
    pub fn into_parts(self) -> (String, String) {
        (self.namespace, self.reference)
    }
}

impl fmt::Display for ChainId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.namespace, self.reference)
    }
}

impl From<ChainId> for String {
    fn from(value: ChainId) -> Self {
        value.to_string()
    }
}

/// Invalid `namespace:reference` string.
#[derive(Debug, thiserror::Error)]
#[error("Invalid chain id format {0}")]
pub struct ChainIdFormatError(
    /// The rejected input.
    pub String,
);

impl FromStr for ChainId {
    type Err = ChainIdFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (namespace, reference) = s
            .split_once(':')
            .ok_or_else(|| ChainIdFormatError(s.into()))?;
        Ok(Self {
            namespace: namespace.into(),
            reference: reference.into(),
        })
    }
}

impl Serialize for ChainId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ChainId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).map_err(de::Error::custom)
    }
}

/// Known network definition with its chain ID and human-readable name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkInfo {
    /// Human-readable network name (e.g. `"base-sepolia"`).
    pub name: &'static str,
    /// CAIP-2 namespace (e.g. `"eip155"`).
    pub namespace: &'static str,
    /// Chain reference (e.g. `"84532"`).
    pub reference: &'static str,
}

impl NetworkInfo {
    /// `ChainId` for this network.
    #[must_use]
    pub fn chain_id(&self) -> ChainId {
        ChainId::new(self.namespace, self.reference)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "unit tests panic on assertion failure")]
mod tests {
    use super::*;

    #[test]
    fn serialize_eip155() {
        let chain_id = ChainId::new("eip155", "1");
        let serialized = serde_json::to_string(&chain_id).unwrap();
        assert_eq!(serialized, "\"eip155:1\"");
    }

    #[test]
    fn serialize_solana() {
        let chain_id = ChainId::new("solana", "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp");
        let serialized = serde_json::to_string(&chain_id).unwrap();
        assert_eq!(serialized, "\"solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp\"");
    }

    #[test]
    fn deserialize_eip155() {
        let chain_id: ChainId = serde_json::from_str("\"eip155:1\"").unwrap();
        assert_eq!(chain_id.namespace(), "eip155");
        assert_eq!(chain_id.reference(), "1");
    }

    #[test]
    fn deserialize_solana() {
        let chain_id: ChainId =
            serde_json::from_str("\"solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp\"").unwrap();
        assert_eq!(chain_id.namespace(), "solana");
        assert_eq!(chain_id.reference(), "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp");
    }

    #[test]
    fn roundtrip_eip155() {
        let original = ChainId::new("eip155", "8453");
        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: ChainId = serde_json::from_str(&serialized).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn roundtrip_solana() {
        let original = ChainId::new("solana", "devnet");
        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: ChainId = serde_json::from_str(&serialized).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn deserialize_invalid_format() {
        let result: Result<ChainId, _> = serde_json::from_str("\"invalid\"");
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_unknown_namespace() {
        let result: Result<ChainId, _> = serde_json::from_str("\"unknown:1\"");
        assert!(result.is_ok());
    }
}
