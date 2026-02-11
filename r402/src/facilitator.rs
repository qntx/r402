//! Core trait and error type for x402 payment facilitators.
//!
//! This module provides the unified [`Facilitator`] trait for verifying and settling
//! x402 payments, along with the [`FacilitatorError`] enum covering all failure modes.
//!
//! The trait is dyn-compatible, allowing heterogeneous facilitator instances to be
//! stored in registries and passed as trait objects.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::proto;
use crate::proto::{AsPaymentProblem, ErrorReason, PaymentProblem, PaymentVerificationError};

/// Errors that can occur during facilitator operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FacilitatorError {
    /// Payment verification failed (invalid signature, insufficient balance, etc.).
    #[error(transparent)]
    PaymentVerification(#[from] PaymentVerificationError),
    /// On-chain operation failed (RPC error, transaction reverted, etc.).
    #[error("Onchain error: {0}")]
    OnchainFailure(String),
    /// A lifecycle hook aborted the operation.
    #[error("{reason}: {message}")]
    Aborted {
        /// Machine-readable abort reason.
        reason: String,
        /// Human-readable abort message.
        message: String,
    },
    /// Any other error not covered by the specific variants.
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl AsPaymentProblem for FacilitatorError {
    fn as_payment_problem(&self) -> PaymentProblem {
        match self {
            Self::PaymentVerification(e) => e.as_payment_problem(),
            Self::OnchainFailure(e) => PaymentProblem::new(ErrorReason::UnexpectedError, e.clone()),
            Self::Aborted { reason, message } => {
                PaymentProblem::new(ErrorReason::UnexpectedError, format!("{reason}: {message}"))
            }
            Self::Other(e) => PaymentProblem::new(ErrorReason::UnexpectedError, e.to_string()),
        }
    }
}

/// Trait defining the asynchronous interface for x402 payment facilitators.
///
/// This is the unified trait for both local scheme handlers (EVM/SVM) and remote
/// facilitator clients (HTTP). It is dyn-compatible, allowing instances to be
/// stored as `Box<dyn Facilitator>` in registries.
pub trait Facilitator: Send + Sync {
    /// Verifies a proposed x402 payment payload against a [`proto::VerifyRequest`].
    ///
    /// This includes checking payload integrity, signature validity, balance sufficiency,
    /// network compatibility, and compliance with the declared payment requirements.
    fn verify(
        &self,
        request: proto::VerifyRequest,
    ) -> Pin<Box<dyn Future<Output = Result<proto::VerifyResponse, FacilitatorError>> + Send + '_>>;

    /// Executes an on-chain x402 settlement for a valid [`proto::SettleRequest`].
    ///
    /// This method should re-validate the payment and, if valid, perform
    /// an onchain call to settle the payment.
    fn settle(
        &self,
        request: proto::SettleRequest,
    ) -> Pin<Box<dyn Future<Output = Result<proto::SettleResponse, FacilitatorError>> + Send + '_>>;

    /// Returns the payment kinds supported by this facilitator.
    fn supported(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<proto::SupportedResponse, FacilitatorError>> + Send + '_>>;
}

impl<T: Facilitator> Facilitator for Arc<T> {
    fn verify(
        &self,
        request: proto::VerifyRequest,
    ) -> Pin<Box<dyn Future<Output = Result<proto::VerifyResponse, FacilitatorError>> + Send + '_>>
    {
        self.as_ref().verify(request)
    }

    fn settle(
        &self,
        request: proto::SettleRequest,
    ) -> Pin<Box<dyn Future<Output = Result<proto::SettleResponse, FacilitatorError>> + Send + '_>>
    {
        self.as_ref().settle(request)
    }

    fn supported(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<proto::SupportedResponse, FacilitatorError>> + Send + '_>>
    {
        self.as_ref().supported()
    }
}
