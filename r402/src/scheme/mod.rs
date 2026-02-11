//! Payment scheme system for x402.
//!
//! This module provides the extensible scheme system that allows different
//! payment methods to be plugged into the x402 protocol. Each scheme defines
//! how payments are authorized, verified, and settled.
//!
//! # Facilitator-Side
//!
//! - [`crate::Facilitator`] - Processes verify/settle requests
//! - [`SchemeBlueprint`] / [`SchemeBuilder`] - Factories that create handlers
//! - [`SchemeRegistry`] - Maps chain+scheme combinations to handlers
//!
//! # Server-Side
//!
//! - [`SchemeServer`] - Converts prices into [`v2::PaymentRequirements`](crate::proto::v2::PaymentRequirements)
//! - [`AssetAmount`] - Resolved token amount
//!
//! # Client-Side
//!
//! - [`SchemeClient`] - Generates [`PaymentCandidate`]s from 402 responses
//! - [`PaymentSelector`] - Chooses the best candidate ([`FirstMatch`], [`PreferChain`], [`MaxAmount`])
//!
//! # Hooks
//!
//! Use [`crate::hooks::FacilitatorHooks`] and [`crate::hooks::HookedFacilitator`]
//! to add lifecycle hooks around verify/settle operations.

mod client;
mod registry;
mod server;

pub use client::*;
pub use registry::*;
pub use server::*;

/// A unit struct representing the string literal `"exact"`.
///
/// This is the canonical scheme name for exact-amount payment schemes
/// across all chain families (EVM, Solana, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExactScheme;

impl ExactScheme {
    /// The string literal value: `"exact"`.
    pub const VALUE: &'static str = "exact";
}

impl std::fmt::Display for ExactScheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(Self::VALUE)
    }
}

impl AsRef<str> for ExactScheme {
    fn as_ref(&self) -> &str {
        Self::VALUE
    }
}

impl std::str::FromStr for ExactScheme {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == Self::VALUE {
            Ok(Self)
        } else {
            Err(format!("expected '{}', got '{s}'", Self::VALUE))
        }
    }
}

impl serde::Serialize for ExactScheme {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(Self::VALUE)
    }
}

impl<'de> serde::Deserialize<'de> for ExactScheme {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        if s == Self::VALUE {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom(format!(
                "expected '{}', got '{s}'",
                Self::VALUE,
            )))
        }
    }
}

/// Trait for identifying a payment scheme.
///
/// Each scheme has a unique identifier composed of the chain namespace
/// and scheme name.
pub trait SchemeId {
    /// Returns the chain namespace (e.g., "eip155", "solana").
    fn namespace(&self) -> &str;
    /// Returns the scheme name (e.g., "exact").
    fn scheme(&self) -> &str;
    /// Returns the CAIP-2 family pattern this scheme supports.
    ///
    /// Used to group signers by blockchain family in the supported response.
    /// The default implementation derives the pattern from [`Self::namespace`].
    ///
    /// # Examples
    ///
    /// - EVM schemes return `"eip155:*"`
    /// - Solana schemes return `"solana:*"`
    fn caip_family(&self) -> String {
        format!("{}:*", self.namespace())
    }
    /// Returns the full scheme identifier (e.g., "eip155-exact").
    fn id(&self) -> String {
        format!("{}-{}", self.namespace(), self.scheme(),)
    }
}
