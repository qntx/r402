//! x402 Payment Protocol SDK for Rust.
//!
//! This crate provides the core types, traits, and abstractions for the x402
//! payment protocol.
//!
//! - [`proto`] — Wire format types (V1/V2), facilitator responses, and helpers
//! - [`scheme`] — Traits for client, server, and facilitator scheme implementations
//! - [`client`] — Async client-side registration, policies, hooks, and payment creation
//! - [`facilitator`] — Async facilitator-side registration, routing, hooks, and supported-kinds
//! - [`server`] — Async server-side resource protection, requirement building, and payment delegation
//! - [`hooks`] — Hook context types and result types for extensibility
//! - [`config`] — Configuration types for resources
//! - [`error`] — Domain-specific error types

pub mod client;
pub mod config;
pub mod error;
pub mod facilitator;
pub mod hooks;
pub mod proto;
pub mod scheme;
pub mod server;

/// Re-export all wire format types from [`proto`] at the crate root.
pub use proto::*;
