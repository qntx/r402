//! EVM "exact" payment scheme implementation.
//!
//! This module implements the `exact` scheme using ERC-3009
//! `transferWithAuthorization` for precise payment amounts.
//!
//! - [`types`] — Wire format types (authorization, payload, sol! bindings)
//! - [`server`] — Server-side price parsing and requirement enhancement
//! - [`client`] — Client-side EIP-712 signing (feature: `client`)
//! - [`facilitator`] — Facilitator-side verify + settle (feature: `facilitator`)

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "facilitator")]
pub mod facilitator;
pub mod server;
pub mod types;

pub use types::*;
