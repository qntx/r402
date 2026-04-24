//! Scheme identifiers, registry, and the extensible payment scheme system.
//!
//! An x402 **scheme** is a strategy for transforming a payment requirement
//! into an on-chain transaction. The two schemes defined by the v2 spec are
//! [`ExactScheme`] (buyer transfers exactly `maxAmountRequired`) and
//! [`UptoScheme`] (buyer authorises up to `maxAmountRequired`; facilitator
//! pulls the actual amount at settle time).
//!
//! Client, server, and facilitator implementations all reference schemes by
//! their `SchemeId` (namespace + scheme name). The registry module wires
//! chains + schemes to [`crate::facilitator::Facilitator`] handlers.

mod client;
mod registry;
pub mod sealed;
mod server;

pub use client::*;
pub use registry::*;
pub use server::*;

/// Identity trait for scheme markers.
///
/// Implemented by name-marker types such as [`ExactScheme`] / [`UptoScheme`]
/// and by concrete scheme handlers provided by chain crates.
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

/// Unit marker representing the string literal `"exact"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExactScheme;

/// Unit marker representing the string literal `"upto"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UptoScheme;

macro_rules! impl_scheme_marker {
    ($ty:ty, $value:literal) => {
        impl $ty {
            /// The canonical wire value.
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
            type Err = String;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                if s == Self::VALUE {
                    Ok(Self)
                } else {
                    Err(format!("expected '{}', got '{s}'", Self::VALUE))
                }
            }
        }

        impl serde::Serialize for $ty {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(Self::VALUE)
            }
        }

        impl<'de> serde::Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(
                deserializer: D,
            ) -> Result<Self, D::Error> {
                let s = <&str>::deserialize(deserializer)?;
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

#[cfg(test)]
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
    fn wrong_scheme_rejected() {
        assert!(serde_json::from_str::<ExactScheme>(r#""upto""#).is_err());
        assert!(serde_json::from_str::<UptoScheme>(r#""exact""#).is_err());
    }
}
