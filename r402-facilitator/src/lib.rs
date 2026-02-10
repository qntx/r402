#![cfg_attr(docsrs, feature(doc_auto_cfg))]

//! x402 Facilitator Server
//!
//! A production-ready HTTP server implementing the [x402](https://www.x402.org) payment protocol.
//!
//! This crate provides a complete, runnable facilitator that supports multiple blockchain
//! networks (EVM/EIP-155 and Solana) and can verify and settle payments on-chain.
//!
//! # Modules
//!
//! - [`chain`] — Blockchain provider abstractions
//! - [`config`] — Configuration types and loading
//! - [`handlers`] — HTTP endpoint handlers for verify, settle, supported
//! - [`run`] — Main server initialization and runtime
//! - [`schemes`] — Scheme builder implementations for supported payment schemes
//! - [`util`] — Utilities for graceful shutdown and telemetry

pub mod chain;
pub mod config;
pub mod handlers;
pub mod local;
pub mod run;
pub mod schemes;
pub mod util;

pub use local::FacilitatorLocal;
pub use run::run;
