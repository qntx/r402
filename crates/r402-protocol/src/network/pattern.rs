//! Chain-ID patterns: exact, wildcard, and set.

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use super::id::{ChainId, ChainIdFormatError};

/// Pattern for matching [`ChainId`] values.
///
/// - Wildcard: `"eip155:*"`
/// - Exact: `"eip155:8453"`
/// - Set: `"eip155:{1,8453,137}"`
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChainIdPattern {
    /// Any chain in `namespace`.
    Wildcard {
        /// Namespace to match (`eip155`, `solana`, …).
        namespace: String,
    },
    /// One specific chain.
    Exact {
        /// Namespace of the chain.
        namespace: String,
        /// Reference of the chain.
        reference: String,
    },
    /// Any reference in `references` within `namespace`.
    Set {
        /// Namespace of the chains.
        namespace: String,
        /// Set of chain references to match.
        references: HashSet<String>,
    },
}

impl ChainIdPattern {
    /// Wildcard matching any chain in `namespace`.
    pub fn wildcard<S: Into<String>>(namespace: S) -> Self {
        Self::Wildcard {
            namespace: namespace.into(),
        }
    }

    /// Exact match on namespace and reference.
    pub fn exact<N: Into<String>, R: Into<String>>(namespace: N, reference: R) -> Self {
        Self::Exact {
            namespace: namespace.into(),
            reference: reference.into(),
        }
    }

    /// Set match on any of `references` in `namespace`.
    pub fn set<N: Into<String>>(namespace: N, references: HashSet<String>) -> Self {
        Self::Set {
            namespace: namespace.into(),
            references,
        }
    }

    /// Whether `chain_id` matches this pattern.
    #[must_use]
    pub fn matches(&self, chain_id: &ChainId) -> bool {
        match self {
            Self::Wildcard { namespace } => chain_id.namespace() == namespace,
            Self::Exact {
                namespace,
                reference,
            } => chain_id.namespace() == namespace && chain_id.reference() == reference,
            Self::Set {
                namespace,
                references,
            } => chain_id.namespace() == namespace && references.contains(chain_id.reference()),
        }
    }

    /// Namespace of this pattern.
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

        if rest == "*" {
            return Ok(Self::wildcard(namespace));
        }

        if let Some(inner) = rest.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
            let items: Vec<&str> = inner.split(',').map(str::trim).collect();
            if items.is_empty() || items.iter().any(|item| item.is_empty()) {
                return Err(ChainIdFormatError(s.into()));
            }
            let references = items.into_iter().map(Into::into).collect();
            return Ok(Self::set(namespace, references));
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_matches() {
        let pattern = ChainIdPattern::wildcard("eip155");
        assert!(pattern.matches(&ChainId::new("eip155", "1")));
        assert!(pattern.matches(&ChainId::new("eip155", "8453")));
        assert!(pattern.matches(&ChainId::new("eip155", "137")));
        assert!(!pattern.matches(&ChainId::new("solana", "mainnet")));
    }

    #[test]
    fn exact_matches() {
        let pattern = ChainIdPattern::exact("eip155", "1");
        assert!(pattern.matches(&ChainId::new("eip155", "1")));
        assert!(!pattern.matches(&ChainId::new("eip155", "8453")));
        assert!(!pattern.matches(&ChainId::new("solana", "1")));
    }

    #[test]
    fn set_matches() {
        let references: HashSet<String> =
            ["1", "8453", "137"].into_iter().map(String::from).collect();
        let pattern = ChainIdPattern::set("eip155", references);
        assert!(pattern.matches(&ChainId::new("eip155", "1")));
        assert!(pattern.matches(&ChainId::new("eip155", "8453")));
        assert!(pattern.matches(&ChainId::new("eip155", "137")));
        assert!(!pattern.matches(&ChainId::new("eip155", "42")));
        assert!(!pattern.matches(&ChainId::new("solana", "1")));
    }

    #[test]
    fn namespace_accessor() {
        let wildcard = ChainIdPattern::wildcard("eip155");
        assert_eq!(wildcard.namespace(), "eip155");

        let exact = ChainIdPattern::exact("solana", "mainnet");
        assert_eq!(exact.namespace(), "solana");

        let references: HashSet<String> = std::iter::once("1").map(String::from).collect();
        let set = ChainIdPattern::set("eip155", references);
        assert_eq!(set.namespace(), "eip155");
    }
}
