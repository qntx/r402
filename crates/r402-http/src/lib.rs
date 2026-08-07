#![cfg_attr(docsrs, feature(doc_cfg))]

//! HTTP transport layer for the x402 payment protocol.
//!
//! This crate provides HTTP middleware for both client and server roles
//! in the x402 payment protocol.
//!
//! # Feature Flags
//!
//! - `server` — Axum/Tower middleware for payment gating
//! - `client` — reqwest-middleware for automatic 402 handling
//! - `telemetry` — Tracing instrumentation

// `compact_str` is consumed by the server paygate's wire types; silence the
// linter when the server feature is disabled.
#[cfg(not(feature = "server"))]
use compact_str as _;
#[cfg(test)]
use tokio as _;
#[cfg(test)]
use wiremock as _;

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "client")]
pub mod client;
