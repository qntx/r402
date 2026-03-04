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
//! - **`V2Eip155ExactClient`** (from `r402-evm`) - EIP-155 chains, "exact" payment scheme
//! - **`V2SolanaExactClient`** (from `r402-svm`) - Solana chains, "exact" payment scheme
//!
//! See [`X402Client::register`] for more details on registering scheme clients.
//!
//! ## Payment Selection
//!
//! When multiple payment options are available, the [`X402Client`] uses a `PaymentSelector`
//! to choose the best option. By default, it uses `FirstMatch` which selects the first
//! matching scheme. You can implement custom selection logic by providing your own selector.
//!
//! See [`X402Client::with_selector`] for custom payment selection.

pub mod hooks;
mod middleware;

pub use hooks::ClientHooks;
pub use middleware::{X402Client, parse_payment_required};
use reqwest::Client;
use reqwest_middleware as rqm;

impl<S> X402Client<S>
where
    Self: rqm::Middleware,
{
    /// Wraps a reqwest [`Client`] with x402 payment middleware.
    ///
    /// Returns a [`rqm::ClientWithMiddleware`] that automatically handles
    /// 402 Payment Required responses using registered scheme clients.
    #[must_use]
    pub fn wrap(self, client: Client) -> rqm::ClientWithMiddleware {
        rqm::ClientBuilder::new(client).with(self).build()
    }

    /// Wraps a reqwest [`Client`] and returns the middleware builder
    /// for further customization (e.g., adding more middleware).
    #[must_use]
    pub fn wrap_builder(self, client: Client) -> rqm::ClientBuilder {
        rqm::ClientBuilder::new(client).with(self)
    }
}

/// Extension trait for adding x402 payment handling to a reqwest [`Client`].
///
/// ```rust,ignore
/// use r402_http::client::{WithPayments, X402Client};
///
/// let client = reqwest::Client::new().with_payments(x402);
/// ```
pub trait WithPayments {
    /// Adds x402 payment middleware, returning a ready-to-use client.
    fn with_payments<S>(self, x402: X402Client<S>) -> rqm::ClientWithMiddleware
    where
        X402Client<S>: rqm::Middleware;
}

impl WithPayments for Client {
    fn with_payments<S>(self, x402: X402Client<S>) -> rqm::ClientWithMiddleware
    where
        X402Client<S>: rqm::Middleware,
    {
        x402.wrap(self)
    }
}
