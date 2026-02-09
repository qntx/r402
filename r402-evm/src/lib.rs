//! EVM chain support for the x402 payment protocol.
//!
//! Provides types, network registries, and scheme implementations
//! for EVM-compatible blockchains.
//!
//! # Modules
//!
//! - [`chain`] — EVM chain primitives (chain IDs, token deployments)
//! - [`networks`] — Known EVM network configurations (Base, Polygon, etc.)
//! - [`exact`] — "exact" payment scheme using ERC-3009

pub mod chain;
pub mod exact;
pub mod networks;
