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
pub mod hooks;
pub mod middleware;
pub mod paygate;
pub mod pricing;
pub mod tracker;
pub mod upto;

pub use facilitator::{
    FacilitatorAuthHeaders, FacilitatorClient, FacilitatorClientError, compute_retry_delay,
};
pub use hooks::{DynPaygateHooks, PaygateHooks, ProtectedRequestOutcome};
pub use middleware::{X402Layer, X402Middleware};
pub use paygate::{
    Paygate, PaygateBuilder, PaygateError, ResourceTemplate, SettlementMode, VerifiedPayment,
    X402_EXPOSED_HEADERS, ensure_expose_headers, reason_to_status, settlement_to_header,
};
pub use pricing::{DynamicPriceTags, PriceTagSource, StaticPriceTags};
pub use tracker::BackgroundSettlementTracker;
pub use upto::{
    SETTLEMENT_OVERRIDES_HEADER, SettlementOverrides, UptoActualAmount,
    marshal_settlement_overrides, resolve_response_settlement_amount, set_settlement_overrides,
    set_settlement_overrides_on_response, settlement_overrides_header_name,
    take_settlement_overrides_header,
};
