//! x402 Payment Protocol SDK for Rust.
//!
//! This crate provides the core traits and abstractions for the x402 payment
//! protocol. It re-exports all wire format types from [`r402_proto`] and adds:
//!
//! - [`scheme`] — Traits for client, server, and facilitator scheme implementations
//! - [`client`] — Client-side registration, policies, and payment creation
//! - [`facilitator`] — Facilitator-side registration, routing, and supported-kinds
//! - [`error`] — Domain-specific error types

pub mod client;
pub mod error;
pub mod facilitator;
pub mod scheme;

/// Re-export all wire format types from `r402-proto`.
pub use r402_proto;
pub use r402_proto::*;
