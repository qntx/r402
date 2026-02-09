//! HTTP transport layer for the x402 payment protocol.
//!
//! Provides header encoding/decoding, constants, and (feature-gated)
//! client/server middleware for the x402 payment protocol.
//!
//! # Modules
//!
//! - [`constants`] — HTTP header names, status codes, default URLs
//! - [`headers`] — Base64 encoding/decoding for x402 HTTP headers
//! - [`error`] — HTTP transport error types
//! - [`facilitator`] — HTTP facilitator client (feature: `client`)
//! - [`client`] — reqwest-middleware for automatic 402 handling (feature: `client`)
//! - [`types`] — HTTP server types (feature: `server`)
//! - [`server`] — Axum/Tower payment gate middleware (feature: `server`)

pub mod constants;
pub mod error;
pub mod headers;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
pub mod facilitator;

#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "server")]
pub mod types;
