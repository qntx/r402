//! Protocol types for x402 payment messages.
//!
//! This module defines the wire format types used in the x402 protocol for
//! communication between buyers, sellers, and facilitators.
//!
//! # Key Types
//!
//! - [`SupportedPaymentKind`] - Describes a payment method supported by a facilitator
//! - [`SupportedResponse`] - Response from facilitator's `/supported` endpoint
//! - [`VerifyRequest`] / [`VerifyResponse`] - Payment verification messages
//! - [`SettleRequest`] / [`SettleResponse`] - Payment settlement messages
//! - [`PaymentVerificationError`] - Errors that can occur during verification
//! - [`PaymentProblem`] - Structured error response for payment failures
//!
//! # Wire Format
//!
//! All types serialize to JSON using camelCase field names. The protocol version
//! is indicated by the `x402Version` field in payment payloads.

mod error;
mod request;
mod response;
mod supported;
pub mod v2;
mod wire;

pub use error::*;
pub use request::*;
pub use response::*;
pub use supported::*;
pub use wire::*;

/// A payment required response.
///
/// This is returned with HTTP 402 status to indicate that payment is required.
/// Currently aliases to the V2 wire format.
pub type PaymentRequired = v2::PaymentRequired;
