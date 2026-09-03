//! Concordium chain primitives, gRPC node access, and facilitator provider.

pub mod account;
pub use account::*;

/// Ed25519 signer bound to a Concordium account.
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod signer;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use signer::*;

/// Node queries (gRPC via `concordium-rust-sdk`, mockable).
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod rpc;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use rpc::*;

/// Facilitator provider: signers + node.
#[cfg(feature = "facilitator")]
pub mod provider;
#[cfg(feature = "facilitator")]
pub use provider::*;

/// V1 sponsored transaction construction.
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod tx;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use tx::*;
