//! Paygate-level lifecycle hooks.
//!
//! Unlike facilitator-level [`FacilitatorHooks`](r402_core::facilitator::FacilitatorHooks)
//! which fire **inside** `verify` / `settle`, these hooks fire at the HTTP
//! paygate layer and let integrators:
//!
//! - bypass payment for API-key holders,
//! - enforce IP allow-lists / KYT checks,
//! - short-circuit the request with a custom error response.

use std::future::Future;
use std::pin::Pin;

use axum_core::body::Body;
use http::{Request, StatusCode};

/// Outcome returned by [`PaygateHooks::on_protected_request`].
#[derive(Debug)]
#[non_exhaustive]
pub enum ProtectedRequestOutcome {
    /// Continue with the standard x402 payment flow.
    Continue,
    /// Bypass payment and forward the request to the downstream service.
    GrantAccess,
    /// Abort with the given status code and optional body.
    Abort {
        /// HTTP status to return.
        status: StatusCode,
        /// Optional response body (served as `text/plain`).
        body: Option<String>,
    },
}

/// Lifecycle hooks for the HTTP paygate layer.
///
/// All methods have no-op defaults so implementors can override only what
/// they need. The trait is `Send + Sync` because the paygate itself is
/// `Send + Sync` and holds a shared reference to the hook object.
pub trait PaygateHooks: Send + Sync {
    /// Fires on every request that reaches a protected route, before the
    /// payment check.
    fn on_protected_request<'a>(
        &'a self,
        _req: &'a Request<Body>,
    ) -> impl Future<Output = ProtectedRequestOutcome> + Send + 'a {
        async { ProtectedRequestOutcome::Continue }
    }

    /// Fires after the facilitator verifies the payment but before the
    /// downstream handler runs. The request is passed by `&mut` so hooks can
    /// attach tenant / payer metadata as extensions.
    fn on_payment_verified<'a>(
        &'a self,
        _req: &'a mut Request<Body>,
    ) -> impl Future<Output = ()> + Send + 'a {
        async {}
    }
}

/// Dyn-compatible shim over [`PaygateHooks`].
///
/// The generic trait uses AFIT (`-> impl Future`) which prevents its direct
/// use behind `dyn`. The [`super::paygate::Paygate`] needs to hold
/// hooks as a trait object so the middleware layer remains non-generic; this
/// shim bridges the gap by boxing the futures.
pub trait DynPaygateHooks: Send + Sync {
    /// Dyn-compatible version of [`PaygateHooks::on_protected_request`].
    fn on_protected_request<'a>(
        &'a self,
        req: &'a Request<Body>,
    ) -> Pin<Box<dyn Future<Output = ProtectedRequestOutcome> + Send + 'a>>;

    /// Dyn-compatible version of [`PaygateHooks::on_payment_verified`].
    fn on_payment_verified<'a>(
        &'a self,
        req: &'a mut Request<Body>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

impl<T> DynPaygateHooks for T
where
    T: PaygateHooks + ?Sized,
{
    fn on_protected_request<'a>(
        &'a self,
        req: &'a Request<Body>,
    ) -> Pin<Box<dyn Future<Output = ProtectedRequestOutcome> + Send + 'a>> {
        Box::pin(<T as PaygateHooks>::on_protected_request(self, req))
    }

    fn on_payment_verified<'a>(
        &'a self,
        req: &'a mut Request<Body>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(<T as PaygateHooks>::on_payment_verified(self, req))
    }
}

/// No-op [`PaygateHooks`] implementation used as the default when no hooks
/// are configured.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopPaygateHooks;

impl PaygateHooks for NoopPaygateHooks {}
