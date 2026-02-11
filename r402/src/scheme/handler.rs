//! Facilitator-side scheme handler traits and errors.
//!
//! This module defines the core abstraction for processing payment verification
//! and settlement on the facilitator side.

use crate::proto;
use crate::proto::{AsPaymentProblem, ErrorReason, PaymentProblem, PaymentVerificationError};

use std::future::Future;
use std::pin::Pin;

/// Trait for scheme handlers that process payment verification and settlement.
///
/// Implementations of this trait handle the core payment processing logic:
/// verifying that payments are valid and settling them on-chain.
pub trait SchemeHandler: Send + Sync {
    /// Verifies a payment authorization without settling it.
    ///
    /// This checks that the payment is properly signed, matches the requirements,
    /// and the payer has sufficient funds.
    fn verify(
        &self,
        request: proto::VerifyRequest,
    ) -> Pin<Box<dyn Future<Output = Result<proto::VerifyResponse, SchemeHandlerError>> + Send + '_>>;

    /// Settles a verified payment on-chain.
    ///
    /// This submits the payment transaction to the blockchain and waits
    /// for confirmation.
    fn settle(
        &self,
        request: proto::SettleRequest,
    ) -> Pin<Box<dyn Future<Output = Result<proto::SettleResponse, SchemeHandlerError>> + Send + '_>>;

    /// Returns the payment methods supported by this handler.
    fn supported(
        &self,
    ) -> Pin<
        Box<dyn Future<Output = Result<proto::SupportedResponse, SchemeHandlerError>> + Send + '_>,
    >;
}

/// Trait for building scheme handlers from chain providers.
///
/// The type parameter `P` represents the chain provider type.
pub trait SchemeHandlerBuilder<P> {
    /// Creates a new scheme handler for the given chain provider.
    ///
    /// # Errors
    ///
    /// Returns an error if the handler cannot be built from the provider.
    fn build(
        &self,
        provider: P,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn SchemeHandler>, Box<dyn std::error::Error>>;
}

/// Errors that can occur during scheme operations.
#[derive(Debug, thiserror::Error)]
pub enum SchemeHandlerError {
    /// Payment verification failed.
    #[error(transparent)]
    PaymentVerification(#[from] PaymentVerificationError),
    /// On-chain operation failed.
    #[error("Onchain error: {0}")]
    OnchainFailure(String),
}

impl AsPaymentProblem for SchemeHandlerError {
    fn as_payment_problem(&self) -> PaymentProblem {
        match self {
            Self::PaymentVerification(e) => e.as_payment_problem(),
            Self::OnchainFailure(e) => PaymentProblem::new(ErrorReason::UnexpectedError, e.clone()),
        }
    }
}
