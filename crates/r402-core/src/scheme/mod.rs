//! Scheme identifiers, registry, and the extensible payment scheme system.
//!
//! An x402 **scheme** is a strategy for transforming a payment requirement
//! into an on-chain transaction. Workspace markers cover the v2 schemes
//! r402 implements or is landing:
//!
//! - [`ExactScheme`] — transfer of at least `amount` (chain binding defines exactness)
//! - [`UptoScheme`] — buyer authorises up to a maximum; settle may charge less
//! - [`BatchSettlementScheme`] — deferred / channel-backed settlement
//! - [`AuthCaptureScheme`] — authorize / capture / void / refund flows
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

/// Unit marker representing the string literal `"batch-settlement"`.
///
/// Capital-backed (or credit-backed) deferred settlement; see
/// `scheme_batch_settlement.md` / EVM binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BatchSettlementScheme;

/// Unit marker representing the string literal `"auth-capture"`.
///
/// Authorize / capture / void / refund; see `scheme_auth_capture.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AuthCaptureScheme;

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
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(Self::VALUE)
            }
        }

        impl<'de> serde::Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                // `Cow` rather than `&str`: scheme markers are also decoded from
                // owned `serde_json::Value` trees (e.g. `TypedVerifyRequest::from_verify`),
                // where no borrowed string is available.
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

    /// Scheme markers must decode from an owned `serde_json::Value` tree, not
    /// only from a borrowing `&str` deserializer. `TypedVerifyRequest::from_verify`
    /// goes through `serde_json::from_value`, which cannot hand out borrowed
    /// strings.
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
}
