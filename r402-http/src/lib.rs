#![cfg_attr(docsrs, feature(doc_auto_cfg))]

//! HTTP transport layer for the x402 payment protocol.
//!
//! This crate provides HTTP middleware for both client and server roles
//! in the x402 payment protocol.
//!
//! # Feature Flags
//!
//! - `server` — Axum/Tower middleware for payment gating (from x402-axum)
//! - `client` — reqwest-middleware for automatic 402 handling (from x402-reqwest)
//! - `telemetry` — Tracing instrumentation

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "client")]
pub mod client;
