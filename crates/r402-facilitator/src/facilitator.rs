//! Unified asynchronous interface for x402 payment facilitators.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use r402_protocol::error::FacilitatorError;
use r402_protocol::payment::{
    SettleRequest, SettleResponse, SupportedResponse, VerifyRequest, VerifyResponse,
};

/// Boxed future alias used by [`DynFacilitator`].
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Unified asynchronous interface for x402 facilitators.
///
/// This trait is **not** dyn-compatible because it uses `async fn` in traits.
/// Callers that need a trait object should use [`DynFacilitator`].
pub trait Facilitator: Send + Sync {
    /// Verifies a payment payload against its requirements.
    fn verify(
        &self,
        request: VerifyRequest,
    ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send;

    /// Settles a previously verified payment on-chain.
    ///
    /// [`r402_protocol::error::ErrorReason::SettlementPending`] must be returned
    /// as `Ok(SettleResponse::Failure { transaction, .. })` with a non-empty
    /// hash. `Err` is never retried by a resource server.
    fn settle(
        &self,
        request: SettleRequest,
    ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send;

    /// Returns the payment kinds supported by this facilitator.
    fn supported(&self)
    -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send;
}

/// Object-safe erasure of [`Facilitator`], suitable for storing as
/// `Box<dyn DynFacilitator>` or `Arc<dyn DynFacilitator>`.
pub trait DynFacilitator: Send + Sync {
    /// See [`Facilitator::verify`].
    fn verify(
        &self,
        request: VerifyRequest,
    ) -> BoxFuture<'_, Result<VerifyResponse, FacilitatorError>>;

    /// See [`Facilitator::settle`].
    fn settle(
        &self,
        request: SettleRequest,
    ) -> BoxFuture<'_, Result<SettleResponse, FacilitatorError>>;

    /// See [`Facilitator::supported`].
    fn supported(&self) -> BoxFuture<'_, Result<SupportedResponse, FacilitatorError>>;
}

impl<T> DynFacilitator for T
where
    T: Facilitator + ?Sized,
{
    fn verify(
        &self,
        request: VerifyRequest,
    ) -> BoxFuture<'_, Result<VerifyResponse, FacilitatorError>> {
        Box::pin(<Self as Facilitator>::verify(self, request))
    }

    fn settle(
        &self,
        request: SettleRequest,
    ) -> BoxFuture<'_, Result<SettleResponse, FacilitatorError>> {
        Box::pin(<Self as Facilitator>::settle(self, request))
    }

    fn supported(&self) -> BoxFuture<'_, Result<SupportedResponse, FacilitatorError>> {
        Box::pin(<Self as Facilitator>::supported(self))
    }
}

impl<T: Facilitator> Facilitator for Arc<T> {
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        self.as_ref().verify(request).await
    }

    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        self.as_ref().settle(request).await
    }

    async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
        self.as_ref().supported().await
    }
}

impl<T: Facilitator> Facilitator for Box<T> {
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        self.as_ref().verify(request).await
    }

    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        self.as_ref().settle(request).await
    }

    async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
        self.as_ref().supported().await
    }
}

impl Facilitator for Box<dyn DynFacilitator> {
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        DynFacilitator::verify(self.as_ref(), request).await
    }

    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        DynFacilitator::settle(self.as_ref(), request).await
    }

    async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
        DynFacilitator::supported(self.as_ref()).await
    }
}

impl Facilitator for Arc<dyn DynFacilitator> {
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        DynFacilitator::verify(self.as_ref(), request).await
    }

    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        DynFacilitator::settle(self.as_ref(), request).await
    }

    async fn supported(&self) -> Result<SupportedResponse, FacilitatorError> {
        DynFacilitator::supported(self.as_ref()).await
    }
}
