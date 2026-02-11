#![cfg_attr(docsrs, feature(doc_auto_cfg))]

//! Core types for the x402 payment protocol.
//!
//! This crate provides the foundational types used throughout the x402 ecosystem
//! for implementing HTTP 402 Payment Required flows. It is designed to be
//! blockchain-agnostic, with chain-specific implementations provided by separate crates.
//!
//! # Overview
//!
//! The x402 protocol enables micropayments over HTTP by leveraging the 402 Payment Required
//! status code. When a client requests a paid resource, the server responds with payment
//! requirements. The client signs a payment authorization, which is verified and settled
//! by a facilitator.
//!
//! # Modules
//!
//! - [`amount`] - Human-readable currency amount parsing
//! - [`chain`] - Blockchain identifiers and provider abstractions (CAIP-2 chain IDs)
//! - [`facilitator`] - Core trait for payment verification and settlement
//! - [`hooks`] - Lifecycle hooks for facilitator verify/settle operations
//! - [`networks`] - Registry of well-known blockchain networks
//! - [`proto`] - Wire format types, encoding utilities, and timestamps
//! - [`scheme`] - Payment scheme system for extensible payment methods
//!
//! # Feature Flags
//!
//! - `telemetry` - Enables tracing instrumentation for debugging and monitoring

pub mod amount;
pub mod chain;
pub mod facilitator;
pub mod hooks;
pub mod networks;
pub mod proto;
pub mod scheme;
