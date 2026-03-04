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
//! ## Settlement Modes
//!
//! - **[`SettlementMode::Sequential`]** (default): verify → execute → settle.
//! - **[`SettlementMode::Concurrent`]**: verify → (settle ∥ execute) → await settle.
//! - **[`SettlementMode::Background`]**: verify → spawn settle (fire-and-forget) → execute → return.

pub mod facilitator;
pub mod layer;
pub mod paygate;
pub mod pricing;

pub use layer::{SettlementMode, X402LayerBuilder, X402Middleware};
pub use paygate::{
    Paygate, PaygateBuilder, PaygateError, ResourceTemplate, VerificationError, VerifiedPayment,
    settlement_to_header,
};
pub use pricing::{DynamicPriceTags, PriceTagSource, StaticPriceTags};
