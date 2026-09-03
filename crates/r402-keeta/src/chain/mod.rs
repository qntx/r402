//! Keeta chain primitives and facilitator provider.
//!
//! - [`KeetaChainReference`] — CAIP-2 `21378` / `1413829460`
//! - [`KeetaAddress`] — `keeta_…` public-key string
//! - [`KeetaTokenDeployment`] — token metadata
//! - [`KeetaChainProvider`] — facilitator fee payers (feature `facilitator`)

pub mod account;
pub use account::*;

/// Keeta chain provider implementation.
#[cfg(feature = "facilitator")]
pub mod provider;
#[cfg(feature = "facilitator")]
pub use provider::*;
