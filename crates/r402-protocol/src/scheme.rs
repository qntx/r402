//! Scheme identifiers and name-marker types.
//!
//! - [`ExactScheme`] — transfer of at least `amount`
//! - [`UptoScheme`] — buyer authorises a maximum; settle may charge less
//! - [`BatchSettlementScheme`] — deferred / channel-backed settlement
//! - [`AuthCaptureScheme`] — authorize / capture / void / refund

use std::fmt::{self, Debug, Display, Formatter};

use compact_str::CompactString;

use crate::network::ChainId;

/// Identity for scheme markers and chain-crate handlers.
pub trait SchemeId {
    /// CAIP-2 namespace (e.g. `"eip155"`, `"solana"`).
    fn namespace(&self) -> &str;
    /// Scheme name (e.g. `"exact"`, `"upto"`).
    fn scheme(&self) -> &str;
    /// CAIP-2 family pattern — defaults to `"{namespace}:*"`.
    fn caip_family(&self) -> String {
        format!("{}:*", self.namespace())
    }
    /// Human-readable identifier — defaults to `"{namespace}-{scheme}"`.
    fn id(&self) -> String {
        format!("{}-{}", self.namespace(), self.scheme())
    }
}

/// Unique identifier for a scheme handler (`chain` + scheme name).
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct SchemeSlug {
    /// Chain this handler operates on.
    pub chain_id: ChainId,
    /// Scheme name (`"exact"`, `"upto"`, …).
    pub name: CompactString,
}

impl SchemeSlug {
    /// Constructs a slug.
    #[must_use]
    pub const fn new(chain_id: ChainId, name: CompactString) -> Self {
        Self { chain_id, name }
    }

    /// Wildcard copy with the reference replaced by `*`.
    #[must_use]
    pub fn as_wildcard(&self) -> Self {
        Self {
            chain_id: ChainId::new(self.chain_id.namespace(), "*"),
            name: self.name.clone(),
        }
    }

    /// Whether the reference is `*`.
    #[must_use]
    pub fn is_wildcard(&self) -> bool {
        self.chain_id.reference() == "*"
    }
}

impl Display for SchemeSlug {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.chain_id.namespace(),
            self.chain_id.reference(),
            self.name,
        )
    }
}

/// Failure parsing a scheme marker from a wire string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("expected '{expected}', got '{got}'")]
pub struct SchemeMarkerError {
    /// Canonical wire value.
    pub expected: &'static str,
    /// Observed value.
    pub got: String,
}

/// Unit marker for the string `"exact"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExactScheme;

/// Unit marker for the string `"upto"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UptoScheme;

/// Unit marker for the string `"batch-settlement"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BatchSettlementScheme;

/// Unit marker for the string `"auth-capture"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AuthCaptureScheme;

macro_rules! impl_scheme_marker {
    ($ty:ty, $value:literal) => {
        impl $ty {
            /// Canonical wire value.
            pub const VALUE: &'static str = $value;
        }

        impl std::fmt::Display for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(Self::VALUE)
            }
        }

        impl AsRef<str> for $ty {
            fn as_ref(&self) -> &str {
                Self::VALUE
            }
        }

        impl std::str::FromStr for $ty {
            type Err = SchemeMarkerError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                if s == Self::VALUE {
                    Ok(Self)
                } else {
                    Err(SchemeMarkerError {
                        expected: Self::VALUE,
                        got: s.to_owned(),
                    })
                }
            }
        }

        impl serde::Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(Self::VALUE)
            }
        }

        impl<'de> serde::Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let s = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
                let s = s.as_ref();
                if s == Self::VALUE {
                    Ok(Self)
                } else {
                    Err(serde::de::Error::custom(format!(
                        "expected '{}', got '{s}'",
                        Self::VALUE
                    )))
                }
            }
        }
    };
}

impl_scheme_marker!(ExactScheme, "exact");
impl_scheme_marker!(UptoScheme, "upto");
impl_scheme_marker!(BatchSettlementScheme, "batch-settlement");
impl_scheme_marker!(AuthCaptureScheme, "auth-capture");

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "unit tests panic on assertion failure")]
mod marker_tests {
    use super::*;

    #[test]
    fn exact_serde_roundtrip() {
        let encoded = serde_json::to_string(&ExactScheme).unwrap();
        assert_eq!(encoded, r#""exact""#);
        let decoded: ExactScheme = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, ExactScheme);
    }

    #[test]
    fn upto_serde_roundtrip() {
        let encoded = serde_json::to_string(&UptoScheme).unwrap();
        assert_eq!(encoded, r#""upto""#);
        let decoded: UptoScheme = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, UptoScheme);
    }

    #[test]
    fn batch_settlement_serde_roundtrip() {
        let encoded = serde_json::to_string(&BatchSettlementScheme).unwrap();
        assert_eq!(encoded, r#""batch-settlement""#);
        let decoded: BatchSettlementScheme = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, BatchSettlementScheme);
    }

    #[test]
    fn auth_capture_serde_roundtrip() {
        let encoded = serde_json::to_string(&AuthCaptureScheme).unwrap();
        assert_eq!(encoded, r#""auth-capture""#);
        let decoded: AuthCaptureScheme = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, AuthCaptureScheme);
    }

    #[test]
    fn wrong_scheme_rejected() {
        assert!(serde_json::from_str::<ExactScheme>(r#""upto""#).is_err());
        assert!(serde_json::from_str::<UptoScheme>(r#""exact""#).is_err());
        assert!(serde_json::from_str::<BatchSettlementScheme>(r#""exact""#).is_err());
        assert!(serde_json::from_str::<AuthCaptureScheme>(r#""batch-settlement""#).is_err());
    }

    #[test]
    fn markers_decode_from_owned_value() {
        let exact: ExactScheme = serde_json::from_value(serde_json::json!("exact")).unwrap();
        assert_eq!(exact, ExactScheme);
        let upto: UptoScheme = serde_json::from_value(serde_json::json!("upto")).unwrap();
        assert_eq!(upto, UptoScheme);
        let batch: BatchSettlementScheme =
            serde_json::from_value(serde_json::json!("batch-settlement")).unwrap();
        assert_eq!(batch, BatchSettlementScheme);
        let auth: AuthCaptureScheme =
            serde_json::from_value(serde_json::json!("auth-capture")).unwrap();
        assert_eq!(auth, AuthCaptureScheme);
        assert!(serde_json::from_value::<ExactScheme>(serde_json::json!("upto")).is_err());
    }

    #[test]
    fn slug_display_and_wildcard() {
        let slug = SchemeSlug::new(ChainId::new("eip155", "8453"), "exact".into());
        assert_eq!(slug.to_string(), "eip155:8453:exact");
        let wild = slug.as_wildcard();
        assert!(wild.is_wildcard());
        assert_eq!(wild.to_string(), "eip155:*:exact");
    }
}
