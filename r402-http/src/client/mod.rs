#![cfg_attr(docsrs, feature(doc_auto_cfg))]

//! Reqwest middleware for automatic [x402](https://www.x402.org) payment handling.
//!
//! This crate provides a [`X402Client`] that can be used as a `reqwest` middleware
//! to automatically handle `402 Payment Required` responses. When a request receives
//! a 402 response, the middleware extracts payment requirements, signs a payment,
//! and retries the request with the payment header.
//!
//! ## Registering Scheme Clients
//!
//! The [`X402Client`] uses a plugin architecture for supporting different payment schemes.
//! Register scheme clients for each chain/network you want to support:
//!
//! - **[`V2Eip155ExactClient`]** - EIP-155 chains, "exact" payment scheme
//! - **[`V2SolanaExactClient`]** - Solana chains, "exact" payment scheme
//!
//! See [`X402Client::register`] for more details on registering scheme clients.
//!
//! ## Payment Selection
//!
//! When multiple payment options are available, the [`X402Client`] uses a [`PaymentSelector`]
//! to choose the best option. By default, it uses [`FirstMatch`] which selects the first
//! matching scheme. You can implement custom selection logic by providing your own selector.
//!
//! See [`X402Client::with_selector`] for custom payment selection.

mod builder;
pub mod hooks;
mod middleware;

pub use builder::*;
pub use hooks::ClientHooks;
pub use middleware::*;
