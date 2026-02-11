//! Payment scheme system for x402.
//!
//! This module provides the extensible scheme system that allows different
//! payment methods to be plugged into the x402 protocol. Each scheme defines
//! how payments are authorized, verified, and settled.
//!
//! # Facilitator-Side
//!
//! - [`SchemeHandler`] - Processes verify/settle requests
//! - [`SchemeBlueprint`] / [`SchemeBlueprints`] - Factories that create handlers
//! - [`SchemeRegistry`] - Maps chain+scheme combinations to handlers
//!
//! # Client-Side
//!
//! - [`X402SchemeClient`] - Generates [`PaymentCandidate`]s from 402 responses
//! - [`PaymentSelector`] - Chooses the best candidate ([`FirstMatch`], [`PreferChain`], [`MaxAmount`])

mod client;
mod handler;
mod registry;

pub use client::*;
pub use handler::*;
pub use registry::*;

/// Trait for identifying a payment scheme.
///
/// Each scheme has a unique identifier composed of the protocol version,
/// chain namespace, and scheme name.
pub trait X402SchemeId {
    /// Returns the x402 protocol version (1 or 2).
    fn x402_version(&self) -> u8 {
        2
    }
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
    /// Returns the full scheme identifier (e.g., "v2-eip155-exact").
    fn id(&self) -> String {
        format!(
            "v{}-{}-{}",
            self.x402_version(),
            self.namespace(),
            self.scheme(),
        )
    }
}
