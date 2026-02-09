//! Local x402 Facilitator server.
//!
//! Provides a local facilitator service implementation for the x402 payment
//! protocol, with Axum route handlers for verify, settle, and supported
//! endpoints.
//!
//! # Modules
//!
//! - [`handlers`] — Axum route handlers and router builder
//! - [`error`] — Facilitator service error types

pub mod error;
pub mod handlers;

pub use handlers::{FacilitatorState, facilitator_router};
