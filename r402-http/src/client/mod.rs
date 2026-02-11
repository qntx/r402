#![cfg_attr(docsrs, feature(doc_auto_cfg))]

//! Reqwest middleware for automatic [x402](https://www.x402.org) payment handling.
//!
//! This crate provides a [`X402Client`] that can be used as a `reqwest` middleware
//! to automatically handle `402 Payment Required` responses. When a request receives
//! a 402 response, the middleware extracts payment requirements, signs a payment,
//! and retries the request with the payment header.
//!
//! ## Registering Scheme Clients
//!
//! The [`X402Client`] uses a plugin architecture for supporting different payment schemes.
//! Register scheme clients for each chain/network you want to support:
//!
//! - **[`V2Eip155ExactClient`]** - EIP-155 chains, "exact" payment scheme
//! - **[`V2SolanaExactClient`]** - Solana chains, "exact" payment scheme
//!
//! See [`X402Client::register`] for more details on registering scheme clients.
//!
//! ## Payment Selection
//!
//! When multiple payment options are available, the [`X402Client`] uses a [`PaymentSelector`]
//! to choose the best option. By default, it uses [`FirstMatch`] which selects the first
//! matching scheme. You can implement custom selection logic by providing your own selector.
//!
//! See [`X402Client::with_selector`] for custom payment selection.

pub mod hooks;
mod middleware;

pub use hooks::ClientHooks;
pub use middleware::*;

use reqwest::{Client, ClientBuilder};
use reqwest_middleware as rqm;

/// Trait for adding x402 payment handling to reqwest clients.
///
/// This trait is implemented on [`Client`] and [`ClientBuilder`], allowing
/// you to create a reqwest client with automatic x402 payment handling.
pub trait ReqwestWithPayments<A, S> {
    /// Adds x402 payment middleware to the client or builder.
    ///
    /// # Arguments
    ///
    /// * `x402_client` - The x402 client configured with scheme handlers
    ///
    /// # Returns
    ///
    /// A builder that can be used to build the final client.
    fn with_payments(self, x402_client: X402Client<S>) -> ReqwestWithPaymentsBuilder<A, S>;
}

impl<S> ReqwestWithPayments<Self, S> for Client {
    fn with_payments(self, x402_client: X402Client<S>) -> ReqwestWithPaymentsBuilder<Self, S> {
        ReqwestWithPaymentsBuilder {
            inner: self,
            x402_client,
        }
    }
}

impl<S> ReqwestWithPayments<Self, S> for ClientBuilder {
    fn with_payments(self, x402_client: X402Client<S>) -> ReqwestWithPaymentsBuilder<Self, S> {
        ReqwestWithPaymentsBuilder {
            inner: self,
            x402_client,
        }
    }
}

/// Builder for creating a reqwest client with x402 middleware.
#[allow(missing_debug_implementations)] // generic A may not implement Debug
pub struct ReqwestWithPaymentsBuilder<A, S> {
    inner: A,
    x402_client: X402Client<S>,
}

/// Trait for building the final client from a [`ReqwestWithPaymentsBuilder`].
pub trait ReqwestWithPaymentsBuild {
    /// The type returned by [`build`]
    type BuildResult;
    /// The type returned by [`builder`]
    type BuilderResult;

    /// Builds the client, consuming the builder.
    fn build(self) -> Self::BuildResult;

    /// Returns the underlying reqwest client builder with middleware added.
    fn builder(self) -> Self::BuilderResult;
}

impl<S> ReqwestWithPaymentsBuild for ReqwestWithPaymentsBuilder<Client, S>
where
    X402Client<S>: rqm::Middleware,
{
    type BuildResult = rqm::ClientWithMiddleware;
    type BuilderResult = rqm::ClientBuilder;

    fn build(self) -> Self::BuildResult {
        self.builder().build()
    }

    fn builder(self) -> Self::BuilderResult {
        rqm::ClientBuilder::new(self.inner).with(self.x402_client)
    }
}

impl<S> ReqwestWithPaymentsBuild for ReqwestWithPaymentsBuilder<ClientBuilder, S>
where
    X402Client<S>: rqm::Middleware,
{
    type BuildResult = Result<rqm::ClientWithMiddleware, reqwest::Error>;
    type BuilderResult = Result<rqm::ClientBuilder, reqwest::Error>;

    fn build(self) -> Self::BuildResult {
        let builder = self.builder()?;
        Ok(builder.build())
    }

    fn builder(self) -> Self::BuilderResult {
        let client = self.inner.build()?;
        Ok(rqm::ClientBuilder::new(client).with(self.x402_client))
    }
}
