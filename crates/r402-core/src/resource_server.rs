//! Resource-server orchestration shared by HTTP and MCP transports.
//!
//! Mirrors the Go `X402ResourceServer` surface used by `go/mcp`:
//! `FindMatchingRequirements`, `VerifyPayment`, `SettlePayment`.
//!
//! This type does **not** own pricing or route maps; it only sequences
//! wire construction against a [`Facilitator`].

use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use crate::error::FacilitatorError;
use crate::facilitator::{DynFacilitator, Facilitator};
use crate::wire::{
    PaymentPayload, PaymentRequirements, SettleRequest, SettleResponse, TypedVerifyRequest, V2,
    VerifyRequest, VerifyResponse, find_matching_requirements,
};

/// Wire payment payload with typed requirements and opaque scheme body.
pub type WirePaymentPayload = PaymentPayload<PaymentRequirements, Value>;

/// Server-side payment orchestrator (Go `X402ResourceServer` subset).
pub struct ResourceServer {
    facilitator: Arc<dyn DynFacilitator>,
}

impl Clone for ResourceServer {
    fn clone(&self) -> Self {
        Self {
            facilitator: Arc::clone(&self.facilitator),
        }
    }
}

impl std::fmt::Debug for ResourceServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceServer").finish_non_exhaustive()
    }
}

impl ResourceServer {
    /// Creates a resource server over any [`Facilitator`].
    #[must_use]
    pub fn new<F>(facilitator: Arc<F>) -> Self
    where
        F: Facilitator + 'static,
    {
        let erased: Arc<dyn DynFacilitator> = facilitator;
        Self {
            facilitator: erased,
        }
    }

    /// Creates from an already-erased facilitator handle.
    #[must_use]
    pub fn from_dyn(facilitator: Arc<dyn DynFacilitator>) -> Self {
        Self { facilitator }
    }

    /// Returns a clone of the inner facilitator handle.
    #[must_use]
    pub fn facilitator(&self) -> Arc<dyn DynFacilitator> {
        Arc::clone(&self.facilitator)
    }

    /// Go `FindMatchingRequirements`.
    ///
    /// Method form matches the official Go `X402ResourceServer` surface even
    /// though matching is pure over the arguments.
    #[must_use]
    #[allow(
        clippy::unused_self,
        reason = "API parity with Go X402ResourceServer.FindMatchingRequirements"
    )]
    pub fn find_matching_requirements<'a>(
        &self,
        available: &'a [PaymentRequirements],
        payload: &WirePaymentPayload,
    ) -> Option<&'a PaymentRequirements> {
        find_matching_requirements(available, &payload.accepted)
    }

    /// Go `VerifyPayment`: builds a verify request and calls the facilitator.
    ///
    /// # Errors
    ///
    /// Propagates facilitator transport / internal errors.
    pub async fn verify_payment(
        &self,
        payload: &WirePaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<VerifyResponse, FacilitatorError> {
        let request = build_verify_request(payload, requirements)?;
        DynFacilitator::verify(self.facilitator.as_ref(), request).await
    }

    /// Go `SettlePayment`: builds a settle request and calls the facilitator.
    ///
    /// # Errors
    ///
    /// Propagates facilitator transport / internal errors.
    pub async fn settle_payment(
        &self,
        payload: &WirePaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<SettleResponse, FacilitatorError> {
        let request = build_settle_request(payload, requirements)?;
        DynFacilitator::settle(self.facilitator.as_ref(), request).await
    }
}

fn build_verify_request(
    payload: &WirePaymentPayload,
    requirements: &PaymentRequirements,
) -> Result<VerifyRequest, FacilitatorError> {
    let typed = TypedVerifyRequest {
        x402_version: V2,
        payment_payload: payload.clone(),
        payment_requirements: requirements.clone(),
    };
    to_verify_request(&typed)
}

fn build_settle_request(
    payload: &WirePaymentPayload,
    requirements: &PaymentRequirements,
) -> Result<SettleRequest, FacilitatorError> {
    let typed = TypedVerifyRequest {
        x402_version: V2,
        payment_payload: payload.clone(),
        payment_requirements: requirements.clone(),
    };
    let verify = to_verify_request(&typed)?;
    Ok(SettleRequest::from(verify.into_json()))
}

fn to_verify_request<T: Serialize>(typed: &T) -> Result<VerifyRequest, FacilitatorError> {
    let json = serde_json::to_value(typed).map_err(FacilitatorError::internal)?;
    Ok(VerifyRequest::from(json))
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::wire::SupportedResponse;

    struct MockFacilitator {
        verifies: AtomicUsize,
        settles: AtomicUsize,
    }

    impl Facilitator for MockFacilitator {
        fn verify(
            &self,
            _request: VerifyRequest,
        ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
            self.verifies.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(VerifyResponse::valid("0xpayer")))
        }

        fn settle(
            &self,
            _request: SettleRequest,
        ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
            self.settles.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(SettleResponse::Success {
                payer: "0xpayer".into(),
                transaction: "0xtx".into(),
                network: "eip155:1".into(),
                amount: Some("1".into()),
                extensions: crate::wire::Extensions::new(),
            }))
        }

        fn supported(
            &self,
        ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(SupportedResponse::default()))
        }
    }

    fn sample_payload() -> WirePaymentPayload {
        let req = PaymentRequirements::new(
            "exact".into(),
            "eip155:1".parse().unwrap(),
            "1".into(),
            "0xa".into(),
            "0xb".into(),
            60,
        );
        WirePaymentPayload::new(req, serde_json::json!({"k": 1}))
    }

    #[tokio::test]
    async fn verify_and_settle_invoke_facilitator() {
        let mock = Arc::new(MockFacilitator {
            verifies: AtomicUsize::new(0),
            settles: AtomicUsize::new(0),
        });
        let rs = ResourceServer::new(Arc::clone(&mock));
        let payload = sample_payload();
        let req = payload.accepted.clone();
        assert!(rs.verify_payment(&payload, &req).await.unwrap().is_valid());
        assert!(
            rs.settle_payment(&payload, &req)
                .await
                .unwrap()
                .is_success()
        );
        assert_eq!(mock.verifies.load(Ordering::SeqCst), 1);
        assert_eq!(mock.settles.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn find_matching_delegates_to_core() {
        let mock = Arc::new(MockFacilitator {
            verifies: AtomicUsize::new(0),
            settles: AtomicUsize::new(0),
        });
        let rs = ResourceServer::new(mock);
        let payload = sample_payload();
        let available = [payload.accepted.clone()];
        assert!(
            rs.find_matching_requirements(&available, &payload)
                .is_some()
        );
    }
}
