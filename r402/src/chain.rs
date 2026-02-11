//! Blockchain-specific types and providers for x402 payment processing.
//!
//! This module provides abstractions for interacting with different blockchain networks
//! in the x402 protocol.
//!
//! - [`ChainId`] - A CAIP-2 compliant chain identifier (e.g., `eip155:8453` for Base)
//! - [`ChainIdPattern`] - Pattern matching for chain IDs (exact, wildcard, or set)
//! - [`ChainRegistry`] - Registry of configured chain providers
//! - [`ChainProviderOps`] - Common operations on chain providers
//! - [`DeployedTokenAmount`] - Token amount paired with deployment info

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use crate::networks;

/// A CAIP-2 compliant blockchain identifier.
///
/// Chain IDs uniquely identify blockchain networks across different ecosystems.
/// The format is `namespace:reference` where:
///
/// - `namespace` identifies the blockchain family (e.g., `eip155`, `solana`)
/// - `reference` identifies the specific chain within that family
///
/// # Serialization
///
/// Serializes to/from a colon-separated string: `"eip155:8453"`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChainId {
    namespace: String,
    reference: String,
}

impl ChainId {
    /// Creates a new chain ID from namespace and reference components.
    pub fn new<N: Into<String>, R: Into<String>>(namespace: N, reference: R) -> Self {
        Self {
            namespace: namespace.into(),
            reference: reference.into(),
        }
    }

    /// Returns the namespace component of the chain ID.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Returns the reference component of the chain ID.
    #[must_use]
    pub fn reference(&self) -> &str {
        &self.reference
    }

    /// Consumes the chain ID and returns its (namespace, reference) components.
    #[must_use]
    pub fn into_parts(self) -> (String, String) {
        (self.namespace, self.reference)
    }

    /// Creates a chain ID from a well-known network name.
    ///
    /// This method looks up the network name in the registry of known networks
    /// (see [`crate::networks`]) and returns the corresponding chain ID.
    #[must_use]
    pub fn from_network_name(network_name: &str) -> Option<Self> {
        networks::chain_id_by_network_name(network_name).cloned()
    }

    /// Returns the well-known network name for this chain ID, if any.
    ///
    /// This is the reverse of [`ChainId::from_network_name`].
    #[must_use]
    pub fn as_network_name(&self) -> Option<&'static str> {
        networks::network_name_by_chain_id(self)
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

/// Error returned when parsing an invalid chain ID string.
///
/// A valid chain ID must be in the format `namespace:reference` where both
/// components are non-empty strings.
#[derive(Debug, thiserror::Error)]
#[error("Invalid chain id format {0}")]
pub struct ChainIdFormatError(String);

impl FromStr for ChainId {
    type Err = ChainIdFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(ChainIdFormatError(s.into()));
        }
        Ok(Self {
            namespace: parts[0].into(),
            reference: parts[1].into(),
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

/// A pattern for matching chain IDs.
///
/// Chain ID patterns allow flexible matching of blockchain networks:
///
/// - **Wildcard**: Matches any chain within a namespace (e.g., `eip155:*` matches all EVM chains)
/// - **Exact**: Matches a specific chain (e.g., `eip155:8453` matches only Base)
/// - **Set**: Matches any chain from a set (e.g., `eip155:{1,8453,137}` matches Ethereum, Base, or Polygon)
///
/// # Serialization
///
/// Patterns serialize to human-readable strings:
/// - Wildcard: `"eip155:*"`
/// - Exact: `"eip155:8453"`
/// - Set: `"eip155:{1,8453,137}"`
#[derive(Debug, Clone)]
pub enum ChainIdPattern {
    /// Matches any chain within the specified namespace.
    Wildcard {
        /// The namespace to match (e.g., `eip155`, `solana`).
        namespace: String,
    },
    /// Matches exactly one specific chain.
    Exact {
        /// The namespace of the chain.
        namespace: String,
        /// The reference of the chain.
        reference: String,
    },
    /// Matches any chain from a set of references within a namespace.
    Set {
        /// The namespace of the chains.
        namespace: String,
        /// The set of chain references to match.
        references: HashSet<String>,
    },
}

impl ChainIdPattern {
    /// Creates a wildcard pattern that matches any chain in the given namespace.
    pub fn wildcard<S: Into<String>>(namespace: S) -> Self {
        Self::Wildcard {
            namespace: namespace.into(),
        }
    }

    /// Creates an exact pattern that matches only the specified chain.
    pub fn exact<N: Into<String>, R: Into<String>>(namespace: N, reference: R) -> Self {
        Self::Exact {
            namespace: namespace.into(),
            reference: reference.into(),
        }
    }

    /// Creates a set pattern that matches any chain from the given set of references.
    pub fn set<N: Into<String>>(namespace: N, references: HashSet<String>) -> Self {
        Self::Set {
            namespace: namespace.into(),
            references,
        }
    }

    /// Check if a `ChainId` matches this pattern.
    ///
    /// - `Wildcard` matches any chain with the same namespace
    /// - `Exact` matches only if both namespace and reference are equal
    /// - `Set` matches if the namespace is equal and the reference is in the set
    #[must_use]
    pub fn matches(&self, chain_id: &ChainId) -> bool {
        match self {
            Self::Wildcard { namespace } => chain_id.namespace == *namespace,
            Self::Exact {
                namespace,
                reference,
            } => chain_id.namespace == *namespace && chain_id.reference == *reference,
            Self::Set {
                namespace,
                references,
            } => chain_id.namespace == *namespace && references.contains(&chain_id.reference),
        }
    }

    /// Returns the namespace of this pattern.
    #[must_use]
    pub fn namespace(&self) -> &str {
        match self {
            Self::Wildcard { namespace }
            | Self::Exact { namespace, .. }
            | Self::Set { namespace, .. } => namespace,
        }
    }
}

impl fmt::Display for ChainIdPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wildcard { namespace } => write!(f, "{namespace}:*"),
            Self::Exact {
                namespace,
                reference,
            } => write!(f, "{namespace}:{reference}"),
            Self::Set {
                namespace,
                references,
            } => {
                let refs: Vec<&str> = references.iter().map(AsRef::as_ref).collect();
                write!(f, "{}:{{{}}}", namespace, refs.join(","))
            }
        }
    }
}

impl FromStr for ChainIdPattern {
    type Err = ChainIdFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (namespace, rest) = s
            .split_once(':')
            .ok_or_else(|| ChainIdFormatError(s.into()))?;

        if namespace.is_empty() {
            return Err(ChainIdFormatError(s.into()));
        }

        // Wildcard: eip155:*
        if rest == "*" {
            return Ok(Self::wildcard(namespace));
        }

        // Set: eip155:{1,2,3}
        if let Some(inner) = rest.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
            let mut references = HashSet::new();

            for item in inner.split(',') {
                let item = item.trim();
                if item.is_empty() {
                    return Err(ChainIdFormatError(s.into()));
                }
                references.insert(item.into());
            }

            if references.is_empty() {
                return Err(ChainIdFormatError(s.into()));
            }

            return Ok(Self::set(namespace, references));
        }

        // Exact: eip155:1
        if rest.is_empty() {
            return Err(ChainIdFormatError(s.into()));
        }

        Ok(Self::exact(namespace, rest))
    }
}

impl Serialize for ChainIdPattern {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ChainIdPattern {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).map_err(de::Error::custom)
    }
}

impl From<ChainId> for ChainIdPattern {
    fn from(chain_id: ChainId) -> Self {
        let (namespace, reference) = chain_id.into_parts();
        Self::exact(namespace, reference)
    }
}

impl From<ChainIdPattern> for Vec<ChainIdPattern> {
    fn from(value: ChainIdPattern) -> Self {
        vec![value]
    }
}

impl From<ChainId> for Vec<ChainId> {
    fn from(value: ChainId) -> Self {
        vec![value]
    }
}

/// Common operations available on all chain providers.
///
/// This trait provides a unified interface for querying chain provider metadata
/// regardless of the underlying blockchain type.
pub trait ChainProviderOps {
    /// Returns the addresses of all configured signers for this chain.
    ///
    /// For EVM chains, these are Ethereum addresses (0x-prefixed hex).
    /// For Solana, these are base58-encoded public keys.
    fn signer_addresses(&self) -> Vec<String>;

    /// Returns the CAIP-2 chain identifier for this provider.
    fn chain_id(&self) -> ChainId;
}

impl<T: ChainProviderOps> ChainProviderOps for Arc<T> {
    fn signer_addresses(&self) -> Vec<String> {
        (**self).signer_addresses()
    }
    fn chain_id(&self) -> ChainId {
        (**self).chain_id()
    }
}

/// Registry of configured chain providers indexed by chain ID.
///
/// # Type Parameters
///
/// - `P` - The chain provider type (e.g., `Eip155ChainProvider` or `SolanaChainProvider`)
#[derive(Debug)]
pub struct ChainRegistry<P>(HashMap<ChainId, P>);

impl<P> ChainRegistry<P> {
    /// Creates a new registry from the given provider map.
    #[must_use]
    pub const fn new(providers: HashMap<ChainId, P>) -> Self {
        Self(providers)
    }
}

impl<P> ChainRegistry<P> {
    /// Looks up a provider by exact chain ID.
    ///
    /// Returns `None` if no provider is configured for the given chain.
    #[must_use]
    pub fn by_chain_id(&self, chain_id: &ChainId) -> Option<&P> {
        self.0.get(chain_id)
    }

    /// Looks up providers by chain ID pattern matching.
    ///
    /// Returns all providers whose chain IDs match the given pattern.
    /// The pattern can be:
    /// - Wildcard: Matches any chain within a namespace (e.g., `eip155:*`)
    /// - Exact: Matches a specific chain (e.g., `eip155:8453`)
    /// - Set: Matches any chain from a set of references (e.g., `eip155:{1,8453,137}`)
    #[must_use]
    pub fn by_chain_id_pattern(&self, pattern: &ChainIdPattern) -> Vec<&P> {
        self.0
            .iter()
            .filter_map(|(chain_id, provider)| pattern.matches(chain_id).then_some(provider))
            .collect()
    }
}

/// A token amount paired with its deployment information.
///
/// This type associates a numeric amount with the token deployment it refers to,
/// enabling type-safe handling of token amounts across different chains and tokens.
///
/// # Type Parameters
///
/// - `TAmount` - The numeric type for the amount (e.g., `U256` for EVM, `u64` for Solana)
/// - `TToken` - The token deployment type containing chain and address information
#[derive(Debug, Clone)]
pub struct DeployedTokenAmount<TAmount, TToken> {
    /// The token amount in the token's smallest unit (e.g., wei for ETH, lamports for SOL).
    pub amount: TAmount,
    /// The token deployment information including chain, address, and decimals.
    pub token: TToken,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::networks::{chain_id_by_network_name, network_name_by_chain_id};

    #[test]
    fn test_chain_id_serialize_eip155() {
        let chain_id = ChainId::new("eip155", "1");
        let serialized = serde_json::to_string(&chain_id).unwrap();
        assert_eq!(serialized, "\"eip155:1\"");
    }

    #[test]
    fn test_chain_id_serialize_solana() {
        let chain_id = ChainId::new("solana", "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp");
        let serialized = serde_json::to_string(&chain_id).unwrap();
        assert_eq!(serialized, "\"solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp\"");
    }

    #[test]
    fn test_chain_id_deserialize_eip155() {
        let chain_id: ChainId = serde_json::from_str("\"eip155:1\"").unwrap();
        assert_eq!(chain_id.namespace(), "eip155");
        assert_eq!(chain_id.reference(), "1");
    }

    #[test]
    fn test_chain_id_deserialize_solana() {
        let chain_id: ChainId =
            serde_json::from_str("\"solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp\"").unwrap();
        assert_eq!(chain_id.namespace(), "solana");
        assert_eq!(chain_id.reference(), "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp");
    }

    #[test]
    fn test_chain_id_roundtrip_eip155() {
        let original = ChainId::new("eip155", "8453");
        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: ChainId = serde_json::from_str(&serialized).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_chain_id_roundtrip_solana() {
        let original = ChainId::new("solana", "devnet");
        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: ChainId = serde_json::from_str(&serialized).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_chain_id_deserialize_invalid_format() {
        let result: Result<ChainId, _> = serde_json::from_str("\"invalid\"");
        assert!(result.is_err());
    }

    #[test]
    fn test_chain_id_deserialize_unknown_namespace() {
        let result: Result<ChainId, _> = serde_json::from_str("\"unknown:1\"");
        assert!(result.is_ok());
    }

    #[test]
    fn test_pattern_wildcard_matches() {
        let pattern = ChainIdPattern::wildcard("eip155");
        assert!(pattern.matches(&ChainId::new("eip155", "1")));
        assert!(pattern.matches(&ChainId::new("eip155", "8453")));
        assert!(pattern.matches(&ChainId::new("eip155", "137")));
        assert!(!pattern.matches(&ChainId::new("solana", "mainnet")));
    }

    #[test]
    fn test_pattern_exact_matches() {
        let pattern = ChainIdPattern::exact("eip155", "1");
        assert!(pattern.matches(&ChainId::new("eip155", "1")));
        assert!(!pattern.matches(&ChainId::new("eip155", "8453")));
        assert!(!pattern.matches(&ChainId::new("solana", "1")));
    }

    #[test]
    fn test_pattern_set_matches() {
        let references: HashSet<String> = vec!["1", "8453", "137"]
            .into_iter()
            .map(String::from)
            .collect();
        let pattern = ChainIdPattern::set("eip155", references);
        assert!(pattern.matches(&ChainId::new("eip155", "1")));
        assert!(pattern.matches(&ChainId::new("eip155", "8453")));
        assert!(pattern.matches(&ChainId::new("eip155", "137")));
        assert!(!pattern.matches(&ChainId::new("eip155", "42")));
        assert!(!pattern.matches(&ChainId::new("solana", "1")));
    }

    #[test]
    fn test_pattern_namespace() {
        let wildcard = ChainIdPattern::wildcard("eip155");
        assert_eq!(wildcard.namespace(), "eip155");

        let exact = ChainIdPattern::exact("solana", "mainnet");
        assert_eq!(exact.namespace(), "solana");

        let references: HashSet<String> = vec!["1"].into_iter().map(String::from).collect();
        let set = ChainIdPattern::set("eip155", references);
        assert_eq!(set.namespace(), "eip155");
    }

    #[test]
    fn test_chain_id_from_network_name() {
        let base = chain_id_by_network_name("base").unwrap();
        assert_eq!(base.namespace(), "eip155");
        assert_eq!(base.reference(), "8453");

        let base_sepolia = chain_id_by_network_name("base-sepolia").unwrap();
        assert_eq!(base_sepolia.namespace(), "eip155");
        assert_eq!(base_sepolia.reference(), "84532");

        let polygon = chain_id_by_network_name("polygon").unwrap();
        assert_eq!(polygon.namespace(), "eip155");
        assert_eq!(polygon.reference(), "137");

        let celo = chain_id_by_network_name("celo").unwrap();
        assert_eq!(celo.namespace(), "eip155");
        assert_eq!(celo.reference(), "42220");

        let solana = chain_id_by_network_name("solana").unwrap();
        assert_eq!(solana.namespace(), "solana");
        assert_eq!(solana.reference(), "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp");

        assert!(chain_id_by_network_name("unknown").is_none());
    }

    #[test]
    fn test_network_name_by_chain_id() {
        let chain_id = ChainId::new("eip155", "8453");
        let network_name = network_name_by_chain_id(&chain_id).unwrap();
        assert_eq!(network_name, "base");

        let celo_chain_id = ChainId::new("eip155", "42220");
        let network_name = network_name_by_chain_id(&celo_chain_id).unwrap();
        assert_eq!(network_name, "celo");

        let celo_sepolia_chain_id = ChainId::new("eip155", "11142220");
        let network_name = network_name_by_chain_id(&celo_sepolia_chain_id).unwrap();
        assert_eq!(network_name, "celo-sepolia");

        let solana_chain_id = ChainId::new("solana", "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp");
        let network_name = network_name_by_chain_id(&solana_chain_id).unwrap();
        assert_eq!(network_name, "solana");

        let unknown_chain_id = ChainId::new("eip155", "999999");
        assert!(network_name_by_chain_id(&unknown_chain_id).is_none());
    }

    #[test]
    fn test_chain_id_as_network_name() {
        let chain_id = ChainId::new("eip155", "8453");
        assert_eq!(chain_id.as_network_name(), Some("base"));

        let celo_chain_id = ChainId::new("eip155", "42220");
        assert_eq!(celo_chain_id.as_network_name(), Some("celo"));

        let solana_chain_id = ChainId::new("solana", "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp");
        assert_eq!(solana_chain_id.as_network_name(), Some("solana"));

        let unknown_chain_id = ChainId::new("eip155", "999999");
        assert!(unknown_chain_id.as_network_name().is_none());
    }
}
