//! TON chain support for x402 payments.
//!
//! - [`TvmChainReference`] — CAIP-2 `-239` / `-3`
//! - [`TvmAddress`] — raw `workchain:hex` (user-friendly forms accepted on parse)
//! - [`TvmTokenDeployment`] — jetton minter metadata
//! - [`TvmChainProvider`] — Highload V3 signer + REST (feature `facilitator`)

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::needless_question_mark,
    reason = "TON address and Highload packing"
)]

/// Core TON chain types (addresses, references, token deployments).
pub mod account;
pub use account::*;

/// Exact-scheme constants (opcodes, timeouts, well-known minters).
pub mod defaults;
pub use defaults::*;

/// Cell / BoC codecs for W5R1, TEP-74, and Highload V3.
pub mod codec;

/// REST client abstraction for TON.
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod rpc;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use rpc::*;

/// TON REST backends and Highload V3 facilitator wallet.
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod provider;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use provider::*;
