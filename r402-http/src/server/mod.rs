#![cfg_attr(docsrs, feature(doc_auto_cfg))]

//! Axum middleware for enforcing [x402](https://www.x402.org) payments on protected routes.
//!
//! This middleware validates incoming payment headers using a configured x402 facilitator,
//! and settles valid payments either before or after request execution (configurable).
//!
//! Returns a `402 Payment Required` response if the request lacks a valid payment.
//!
//! See [`X402Middleware`] for full configuration options.
//! For low-level interaction with the facilitator, see [`facilitator_client::FacilitatorClient`].
//!
//! ## Protocol Support
//!
//! Supports both V1 and V2 x402 protocols through the [`PaygateProtocol`] trait.
//! The protocol version is determined by the price tag type used.
//!
//! ## Settlement Timing
//!
//! By default, settlement occurs **after** the request is processed. You can change this behavior:
//!
//! - **[`X402Middleware::settle_before_execution`]** - Settle payment **before** request execution.
//!   This prevents issues where failed settlements need retry or authorization expires.
//! - **[`X402Middleware::settle_after_execution`]** - Settle payment **after** request execution (default).
//!   This allows processing the request before committing the payment on-chain.
//!
//! ## Configuration Notes
//!
//! - **[`X402Middleware::with_price_tag`]** sets the assets and amounts accepted for payment (static pricing).
//! - **[`X402Middleware::with_dynamic_price`]** sets a callback for dynamic pricing based on request context.
//! - **[`X402Middleware::with_base_url`]** sets the base URL for computing full resource URLs.
//!   If not set, defaults to `http://localhost/` (avoid in production).
//! - **[`X402Middleware::with_supported_cache_ttl`]** configures the TTL for caching facilitator capabilities.
//! - **[`X402LayerBuilder::with_description`]** is optional but helps the payer understand what is being paid for.
//! - **[`X402LayerBuilder::with_mime_type`]** sets the MIME type of the protected resource (default: `application/json`).
//! - **[`X402LayerBuilder::with_resource`]** explicitly sets the full URI of the protected resource.

pub mod error;
pub mod facilitator_client;
pub mod layer;
pub mod paygate;
pub mod price_source;
pub mod protocol;

pub use error::{PaygateError, VerificationError};
pub use layer::{X402LayerBuilder, X402Middleware};
pub use price_source::{DynamicPriceTags, PriceTagSource, StaticPriceTags};
pub use protocol::PaygateProtocol;

// Re-export hook types from r402 core for convenience.
pub use r402::hooks::{FacilitatorHooks, SettleContext, VerifyContext};
