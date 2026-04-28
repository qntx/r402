//! Typestate lifecycle for a single x402 payment.
//!
//! A payment moves through three well-defined states:
//!
//! ```text
//! Payment<Unverified> ──verify()──▶ Payment<Verified> ──settle()──▶ Payment<Settled>
//! ```
//!
//! Each transition is a method on the corresponding state; the compiler
//! enforces the order (you cannot call `settle` on an `Unverified` payment).
//! All state types are zero-sized so the `Payment<S>` wrapper compiles to the
//! same memory layout as its payload.
//!
//! # Example
//!
//! ```no_run
//! use r402_core::payment::Payment;
//! use r402_core::Facilitator;
//! use r402_core::wire::VerifyRequest;
//!
//! # async fn run<F: Facilitator>(facilitator: F, payload: VerifyRequest) {
//! let payment = Payment::new(payload.clone());
//! let (verified, _vr) = payment.verify(&facilitator, payload.clone()).await.unwrap();
//! let (_settled, _sr) = verified.settle(&facilitator, payload.into()).await.unwrap();
//! # }
//! ```

use std::marker::PhantomData;

use crate::error::FacilitatorError;
use crate::facilitator::Facilitator;
use crate::wire::{SettleRequest, SettleResponse, VerifyRequest, VerifyResponse};

/// Marker indicating the payment has not yet been verified.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unverified;

/// Marker indicating the payment has passed verification.
#[derive(Debug, Clone, Copy, Default)]
pub struct Verified;

/// Marker indicating the payment has been settled.
#[derive(Debug, Clone, Copy, Default)]
pub struct Settled;

/// A payment moving through the verify → settle lifecycle.
///
/// The type parameter `S` tracks the current state at compile time. The
/// struct is transparent (zero overhead) around the underlying request.
#[derive(Debug, Clone)]
pub struct Payment<S> {
    request: VerifyRequest,
    state: PhantomData<fn() -> S>,
}

impl<S> Payment<S> {
    /// Returns a reference to the wire-level verify request this payment
    /// was constructed from.
    #[must_use]
    pub const fn request(&self) -> &VerifyRequest {
        &self.request
    }
}

impl Payment<Unverified> {
    /// Wraps a raw verify request as an unverified payment.
    #[must_use]
    pub const fn new(request: VerifyRequest) -> Self {
        Self {
            request,
            state: PhantomData,
        }
    }

    /// Runs facilitator verification.
    ///
    /// On success, returns the verified payment together with the
    /// facilitator's [`VerifyResponse`]. On failure, the payment is
    /// consumed and a [`FacilitatorError`] is returned.
    ///
    /// # Errors
    ///
    /// Propagates any [`FacilitatorError`] raised by the underlying
    /// facilitator. A `Valid` verification that returns `VerifyResponse::Invalid`
    /// is surfaced as `Err(FacilitatorError::Verification(VerificationError::InvalidFormat))`.
    pub async fn verify<F: Facilitator>(
        self,
        facilitator: &F,
        request: VerifyRequest,
    ) -> Result<(Payment<Verified>, VerifyResponse), FacilitatorError> {
        let response = facilitator.verify(request).await?;
        match &response {
            VerifyResponse::Valid { .. } => Ok((
                Payment {
                    request: self.request,
                    state: PhantomData,
                },
                response,
            )),
            VerifyResponse::Invalid {
                reason, message, ..
            } => Err(FacilitatorError::Verification(
                crate::error::VerificationError::InvalidFormat(format!(
                    "{reason}: {}",
                    message.as_deref().unwrap_or(""),
                )),
            )),
        }
    }
}

impl Payment<Verified> {
    /// Runs facilitator settlement on a previously verified payment.
    ///
    /// # Errors
    ///
    /// Propagates any [`FacilitatorError`] raised by the facilitator; a
    /// `SettleResponse::Failure` is surfaced as
    /// `Err(FacilitatorError::Onchain(...))`.
    pub async fn settle<F: Facilitator>(
        self,
        facilitator: &F,
        request: SettleRequest,
    ) -> Result<(Payment<Settled>, SettleResponse), FacilitatorError> {
        let response = facilitator.settle(request).await?;
        match &response {
            SettleResponse::Success { .. } => Ok((
                Payment {
                    request: self.request,
                    state: PhantomData,
                },
                response,
            )),
            SettleResponse::Failure { message, .. } => Err(FacilitatorError::Onchain(
                message.as_deref().unwrap_or("").to_owned(),
            )),
        }
    }
}

impl Payment<Settled> {
    /// Extracts the underlying wire-level request.
    #[must_use]
    pub fn into_request(self) -> VerifyRequest {
        self.request
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error_reason::ErrorReason;
    use crate::wire::{Extensions, SettleResponse, SupportedResponse, VerifyResponse};

    struct AlwaysValid;

    impl Facilitator for AlwaysValid {
        async fn verify(
            &self,
            _request: VerifyRequest,
        ) -> Result<VerifyResponse, FacilitatorError> {
            Ok(VerifyResponse::valid("0xPAYER"))
        }

        async fn settle(
            &self,
            _request: SettleRequest,
        ) -> Result<SettleResponse, FacilitatorError> {
            Ok(SettleResponse::Success {
                payer: "0xPAYER".into(),
                transaction: "0xTX".into(),
                network: "eip155:1".into(),
                amount: Some("1000000".into()),
                extensions: Extensions::new(),
            })
        }

        async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
            Ok(SupportedResponse::default())
        }
    }

    struct AlwaysInvalid;

    impl Facilitator for AlwaysInvalid {
        async fn verify(
            &self,
            _request: VerifyRequest,
        ) -> Result<VerifyResponse, FacilitatorError> {
            Ok(VerifyResponse::invalid(None, ErrorReason::InvalidPayload))
        }

        async fn settle(
            &self,
            _request: SettleRequest,
        ) -> Result<SettleResponse, FacilitatorError> {
            unreachable!()
        }

        async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
            Ok(SupportedResponse::default())
        }
    }

    fn dummy_request() -> VerifyRequest {
        serde_json::json!({}).into()
    }

    #[tokio::test]
    async fn verify_then_settle_succeeds() {
        let facilitator = AlwaysValid;
        let payment = Payment::new(dummy_request());
        let (verified, verify_resp) = payment.verify(&facilitator, dummy_request()).await.unwrap();
        assert!(verify_resp.is_valid());
        let (_settled, settle_resp) = verified
            .settle(&facilitator, dummy_request().into())
            .await
            .unwrap();
        assert!(settle_resp.is_success());
    }

    #[tokio::test]
    async fn verify_invalid_propagates() {
        let facilitator = AlwaysInvalid;
        let payment = Payment::new(dummy_request());
        let err = payment
            .verify(&facilitator, dummy_request())
            .await
            .unwrap_err();
        assert!(matches!(err, FacilitatorError::Verification(_)));
    }
}
