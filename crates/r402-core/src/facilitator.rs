//! Unified asynchronous interface for x402 payment facilitators.
//!
//! The core [`Facilitator`] trait uses Rust's native AFIT (async fn in traits)
//! for **zero-cost static dispatch**. For use cases that need a dyn-compatible
//! trait (heterogeneous registries, hook pipelines), the
//! [`DynFacilitator`] trait erases the concrete future type via
//! `Pin<Box<dyn Future>>`. A blanket implementation turns every
//! `Facilitator` into a `DynFacilitator` automatically.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub use crate::error::FacilitatorError;
use crate::wire::{
    SettleRequest, SettleResponse, SupportedResponse, VerifyRequest, VerifyResponse,
};

/// Boxed future alias used by [`DynFacilitator`] and legacy code.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Unified asynchronous interface for x402 facilitators.
///
/// This trait is **not** dyn-compatible because it uses `async fn` in traits.
/// Callers that need a trait object should use [`DynFacilitator`] instead
/// (the blanket impl below makes every `Facilitator` automatically satisfy it).
pub trait Facilitator: Send + Sync {
    /// Verifies a payment payload against its requirements.
    fn verify(
        &self,
        request: VerifyRequest,
    ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send;

    /// Settles a previously verified payment on-chain.
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
