//! Axum middleware for enforcing [x402](https://www.x402.org) payments on protected routes (V2-only).
//!
//! This middleware validates incoming payment headers using a configured x402 facilitator,
//! verifies the payment, executes the request, and settles valid payments after successful
//! execution. If the handler returns an error (4xx/5xx), settlement is skipped.
//!
//! Returns a `402 Payment Required` response if the request lacks a valid payment.
//!
//! See [`X402Middleware`] for full configuration options.
//! For low-level interaction with the facilitator, see [`facilitator::FacilitatorClient`].
//!
//! ## Configuration Notes
//!
//! - **[`X402Middleware::with_price_tag`]** sets the assets and amounts accepted for payment (static pricing).
//! - **[`X402Middleware::with_dynamic_price`]** sets a callback for dynamic pricing based on request context.
//! - **[`X402Middleware::with_base_url`]** sets the base URL for computing full resource URLs.
//!   If not set, defaults to `http://localhost/` (avoid in production).
//! - **[`X402Middleware::with_supported_cache_ttl`]** configures the TTL for caching facilitator capabilities.
//! - **[`X402Middleware::with_facilitator_timeout`]** sets a per-request timeout for facilitator HTTP calls.
//! - **[`X402LayerBuilder::with_description`]** is optional but helps the payer understand what is being paid for.
//! - **[`X402LayerBuilder::with_mime_type`]** sets the MIME type of the protected resource (default: `application/json`).
//! - **[`X402LayerBuilder::with_resource`]** explicitly sets the full URI of the protected resource.

pub mod facilitator;
pub mod layer;
pub mod paygate;
pub mod pricing;

pub use layer::{X402LayerBuilder, X402Middleware};
pub use pricing::{DynamicPriceTags, PriceTagSource, StaticPriceTags};

/// Common verification errors shared between protocol versions.
#[derive(Debug, thiserror::Error)]
pub enum VerificationError {
    /// Required payment header is missing.
    #[error("{0} header is required")]
    PaymentHeaderRequired(&'static str),
    /// Payment header is present but malformed.
    #[error("Invalid or malformed payment header")]
    InvalidPaymentHeader,
    /// No matching payment requirements found.
    #[error("Unable to find matching payment requirements")]
    NoPaymentMatching,
    /// Verification with facilitator failed.
    #[error("Verification failed: {0}")]
    VerificationFailed(String),
}

/// Paygate error type that wraps verification and settlement errors.
#[derive(Debug, thiserror::Error)]
pub enum PaygateError {
    /// Payment verification failed.
    #[error(transparent)]
    Verification(#[from] VerificationError),
    /// On-chain settlement failed.
    #[error("Settlement failed: {0}")]
    Settlement(String),
}
