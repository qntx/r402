//! HTTP client hooks — re-exports of [`r402_core::client`] lifecycle types.
//!
//! Prefer implementing [`r402_core::ClientHooks`] directly. Payment creation
//! is transport-agnostic (signed base64 payload); HTTP adapts that into the
//! `Payment-Signature` header.

pub use r402_core::client::{
    ClientHooks, CreatedPayment, DynClientHooks, PaymentCreationContext, PaymentResponseContext,
    PaymentResponseResult,
};
