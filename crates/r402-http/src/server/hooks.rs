//! HTTP-layer lifecycle hooks (`GateHooks`).

use std::future::Future;
use std::pin::Pin;

use axum_core::body::Body;
use http::{Request, StatusCode};

/// Outcome of [`GateHooks::on_protected_request`].
#[derive(Debug)]
#[non_exhaustive]
pub enum ProtectedRequestOutcome {
    /// Continue with the x402 payment check.
    Continue,
    /// Skip payment and forward the request (SIWX fills this slot later).
    GrantAccess,
    /// Abort with a custom status and optional `text/plain` body.
    Abort {
        /// HTTP status to return.
        status: StatusCode,
        /// Optional response body.
        body: Option<String>,
    },
}

/// HTTP-layer hooks. Defaults are no-ops; [`ProtectedRequestOutcome::GrantAccess`] is an empty slot.
pub trait GateHooks: Send + Sync {
    /// Fires on every protected request, before the payment check.
    fn on_protected_request<'a>(
        &'a self,
        _req: &'a Request<Body>,
    ) -> impl Future<Output = ProtectedRequestOutcome> + Send + 'a {
        async { ProtectedRequestOutcome::Continue }
    }

    /// Fires after a payment is verified, before the inner handler.
    fn on_payment_verified<'a>(
        &'a self,
        _req: &'a mut Request<Body>,
    ) -> impl Future<Output = ()> + Send + 'a {
        async {}
    }
}

/// Object-safe erasure of [`GateHooks`].
pub trait DynGateHooks: Send + Sync {
    /// See [`GateHooks::on_protected_request`].
    fn on_protected_request<'a>(
        &'a self,
        req: &'a Request<Body>,
    ) -> Pin<Box<dyn Future<Output = ProtectedRequestOutcome> + Send + 'a>>;

    /// See [`GateHooks::on_payment_verified`].
    fn on_payment_verified<'a>(
        &'a self,
        req: &'a mut Request<Body>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

impl<T> DynGateHooks for T
where
    T: GateHooks + ?Sized,
{
    fn on_protected_request<'a>(
        &'a self,
        req: &'a Request<Body>,
    ) -> Pin<Box<dyn Future<Output = ProtectedRequestOutcome> + Send + 'a>> {
        Box::pin(<T as GateHooks>::on_protected_request(self, req))
    }

    fn on_payment_verified<'a>(
        &'a self,
        req: &'a mut Request<Body>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(<T as GateHooks>::on_payment_verified(self, req))
    }
}
