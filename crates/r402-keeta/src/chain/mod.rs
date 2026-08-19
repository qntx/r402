//! Keeta chain support for x402 payments.
//!
//! - [`KeetaChainReference`] — CAIP-2 `21378` / `1413829460`
//! - [`KeetaAddress`] — `keeta_…` public-key string
//! - [`KeetaTokenDeployment`] — token metadata
//! - [`KeetaChainProvider`] — facilitator fee payers (feature `facilitator`)

/// Core Keeta chain types (addresses, references, token deployments).
pub mod types;
pub use types::*;

/// Keeta chain provider implementation.
#[cfg(feature = "facilitator")]
pub mod provider;
#[cfg(feature = "facilitator")]
pub use provider::*;
