//! Hook result types and context types for the x402 payment protocol.
//!
//! Provides extensibility points for client, facilitator, and server
//! operations via before/after/failure hooks.
//!
//! Corresponds to Python SDK's `schemas/hooks.py`.

use crate::proto::{
    PaymentPayload, PaymentPayloadV1, PaymentRequired, PaymentRequiredV1, PaymentRequirements,
    PaymentRequirementsV1, SettleResponse, VerifyResponse,
};

/// Return from a before-hook to abort the operation.
#[derive(Debug, Clone)]
pub struct AbortResult {
    /// Human-readable reason for aborting.
    pub reason: String,
}

impl AbortResult {
    /// Creates a new abort result.
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

/// Return from a client failure hook to recover with a payload.
#[derive(Debug, Clone)]
pub enum RecoveredPayloadResult {
    /// Recovered V2 payload.
    V2(Box<PaymentPayload>),
    /// Recovered V1 payload.
    V1(PaymentPayloadV1),
}

/// Return from a verify failure hook to recover with a result.
#[derive(Debug, Clone)]
pub struct RecoveredVerifyResult {
    /// The recovered verify response.
    pub result: VerifyResponse,
}

/// Return from a settle failure hook to recover with a result.
#[derive(Debug, Clone)]
pub struct RecoveredSettleResult {
    /// The recovered settle response.
    pub result: SettleResponse,
}

/// Context for payment creation hooks (client-side).
#[derive(Debug, Clone)]
pub struct PaymentCreationContext {
    /// The 402 response from the server.
    pub payment_required: PaymentRequiredView,
    /// The selected payment requirements.
    pub selected_requirements: RequirementsView,
}

/// Context passed to after-payment-creation hooks.
#[derive(Debug, Clone)]
pub struct PaymentCreatedContext {
    /// The 402 response from the server.
    pub payment_required: PaymentRequiredView,
    /// The selected payment requirements.
    pub selected_requirements: RequirementsView,
    /// The created payment payload.
    pub payment_payload: PayloadView,
}

/// Context passed to payment-creation-failure hooks.
#[derive(Debug, Clone)]
pub struct PaymentCreationFailureContext {
    /// The 402 response from the server.
    pub payment_required: PaymentRequiredView,
    /// The selected payment requirements.
    pub selected_requirements: RequirementsView,
    /// Description of the error that caused the failure.
    pub error: String,
}

/// Context for verify hooks (facilitator-side).
#[derive(Debug, Clone)]
pub struct VerifyContext {
    /// The payment payload being verified.
    pub payment_payload: PayloadView,
    /// The requirements being verified against.
    pub requirements: RequirementsView,
    /// Raw payload bytes (escape hatch for extensions).
    pub payload_bytes: Option<Vec<u8>>,
    /// Raw requirements bytes (escape hatch for extensions).
    pub requirements_bytes: Option<Vec<u8>>,
}

/// Context passed to after-verify hooks.
#[derive(Debug, Clone)]
pub struct VerifyResultContext {
    /// The payment payload that was verified.
    pub payment_payload: PayloadView,
    /// The requirements verified against.
    pub requirements: RequirementsView,
    /// Raw payload bytes.
    pub payload_bytes: Option<Vec<u8>>,
    /// Raw requirements bytes.
    pub requirements_bytes: Option<Vec<u8>>,
    /// The verification result.
    pub result: VerifyResponse,
}

/// Context passed to verify failure hooks.
#[derive(Debug, Clone)]
pub struct VerifyFailureContext {
    /// The payment payload that failed verification.
    pub payment_payload: PayloadView,
    /// The requirements verified against.
    pub requirements: RequirementsView,
    /// Raw payload bytes.
    pub payload_bytes: Option<Vec<u8>>,
    /// Raw requirements bytes.
    pub requirements_bytes: Option<Vec<u8>>,
    /// Description of the error.
    pub error: String,
}

/// Context for settle hooks (facilitator-side).
#[derive(Debug, Clone)]
pub struct SettleContext {
    /// The payment payload being settled.
    pub payment_payload: PayloadView,
    /// The requirements for settlement.
    pub requirements: RequirementsView,
    /// Raw payload bytes.
    pub payload_bytes: Option<Vec<u8>>,
    /// Raw requirements bytes.
    pub requirements_bytes: Option<Vec<u8>>,
}

/// Context passed to after-settle hooks.
#[derive(Debug, Clone)]
pub struct SettleResultContext {
    /// The payment payload that was settled.
    pub payment_payload: PayloadView,
    /// The requirements for settlement.
    pub requirements: RequirementsView,
    /// Raw payload bytes.
    pub payload_bytes: Option<Vec<u8>>,
    /// Raw requirements bytes.
    pub requirements_bytes: Option<Vec<u8>>,
    /// The settlement result.
    pub result: SettleResponse,
}

/// Context passed to settle failure hooks.
#[derive(Debug, Clone)]
pub struct SettleFailureContext {
    /// The payment payload that failed settlement.
    pub payment_payload: PayloadView,
    /// The requirements for settlement.
    pub requirements: RequirementsView,
    /// Raw payload bytes.
    pub payload_bytes: Option<Vec<u8>>,
    /// Raw requirements bytes.
    pub requirements_bytes: Option<Vec<u8>>,
    /// Description of the error.
    pub error: String,
}

/// Version-agnostic view of a payment payload.
#[derive(Debug, Clone)]
pub enum PayloadView {
    /// V2 payload.
    V2(Box<PaymentPayload>),
    /// V1 payload.
    V1(PaymentPayloadV1),
}

/// Version-agnostic view of a payment required response.
#[derive(Debug, Clone)]
pub enum PaymentRequiredView {
    /// V2 payment required.
    V2(PaymentRequired),
    /// V1 payment required.
    V1(PaymentRequiredV1),
}

/// Version-agnostic view of payment requirements.
#[derive(Debug, Clone)]
pub enum RequirementsView {
    /// V2 requirements.
    V2(PaymentRequirements),
    /// V1 requirements.
    V1(PaymentRequirementsV1),
}

impl RequirementsView {
    /// Returns the scheme identifier.
    #[must_use]
    pub fn scheme(&self) -> &str {
        match self {
            Self::V2(r) => &r.scheme,
            Self::V1(r) => &r.scheme,
        }
    }

    /// Returns the network identifier.
    #[must_use]
    pub fn network(&self) -> &str {
        match self {
            Self::V2(r) => &r.network,
            Self::V1(r) => &r.network,
        }
    }

    /// Returns the payment amount as a string.
    #[must_use]
    pub fn amount(&self) -> &str {
        match self {
            Self::V2(r) => r.amount(),
            Self::V1(r) => r.amount(),
        }
    }
}
