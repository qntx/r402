//! HTTP access-grant for `sign-in-with-x`.
//!
//! Valid CAIP-122 proof plus (`auth_only` or paid-address hit) skips payment
//! and never calls facilitator `/verify`. Failed proofs continue as unpaid
//! 402 with a fresh challenge.

use std::future::Future;
use std::sync::Arc;

use axum_core::body::Body;
use http::Request;
use r402_extensions::siwx::{
    PaidAddressStore, SIWX_KEY, SiwxChain, SiwxError, SiwxExtension, SiwxOrigin, SiwxProof,
};
use r402_protocol::payment::ExtensionEntry;

use super::hooks::{GateHooks, ProtectedRequestOutcome};
use crate::headers::SIGN_IN_WITH_X;

/// Resource-server SIWX configuration used by the HTTP gate.
pub struct SiwxGate {
    extension: SiwxExtension,
    store: Arc<dyn PaidAddressStore>,
    auth_only: bool,
}

impl Clone for SiwxGate {
    fn clone(&self) -> Self {
        Self {
            extension: self.extension.clone(),
            store: Arc::clone(&self.store),
            auth_only: self.auth_only,
        }
    }
}

impl std::fmt::Debug for SiwxGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SiwxGate")
            .field("origin", self.extension.origin())
            .field("auth_only", &self.auth_only)
            .finish_non_exhaustive()
    }
}

impl SiwxGate {
    /// Binds SIWX to a configured public origin and paid-address store.
    ///
    /// Origin is required. There is no `Host` constructor.
    #[must_use]
    pub fn new(origin: SiwxOrigin, store: impl PaidAddressStore + 'static) -> Self {
        Self {
            extension: SiwxExtension::new(origin),
            store: Arc::new(store),
            auth_only: false,
        }
    }

    /// Adds a supported authentication chain.
    #[must_use]
    pub fn with_chain(mut self, chain: SiwxChain) -> Self {
        self.extension = self.extension.with_chain(chain);
        self
    }

    /// Sets the CAIP-122 statement shown to the wallet.
    #[must_use]
    pub fn with_statement(mut self, statement: impl Into<compact_str::CompactString>) -> Self {
        self.extension = self.extension.with_statement(statement);
        self
    }

    /// Grants access on valid signature even when the address has not paid.
    #[must_use]
    pub const fn with_auth_only(mut self) -> Self {
        self.auth_only = true;
        self
    }

    /// Whether empty price tags must not bypass the gate.
    #[must_use]
    pub const fn is_auth_only(&self) -> bool {
        self.auth_only
    }

    /// Configured public origin (never `Host`).
    #[must_use]
    pub const fn origin(&self) -> &SiwxOrigin {
        self.extension.origin()
    }

    /// Shared paid-address / nonce store.
    #[must_use]
    pub fn store(&self) -> &dyn PaidAddressStore {
        self.store.as_ref()
    }

    /// Per-request 402 challenge entry (fresh nonce and timestamps).
    ///
    /// # Errors
    ///
    /// [`SiwxError`] when nonce or timestamp formatting fails.
    pub fn challenge_entry(&self, path: &str) -> Result<ExtensionEntry, SiwxError> {
        self.extension.challenge_now(path)
    }

    /// Records a successful settlement against configured origin + `path`.
    pub fn record_success(&self, path: &str, address: &str) {
        let key = self.extension.origin().store_key(path);
        self.store.record_success(&key, address);
    }

    /// Wire key inserted on `PaymentRequired.extensions`.
    #[must_use]
    pub const fn key() -> &'static str {
        SIWX_KEY
    }

    pub(crate) async fn try_grant(&self, header: &str, path: &str) -> bool {
        let Ok(proof) = SiwxProof::parse_header(header) else {
            return false;
        };
        if self.store.has_used_nonce(&proof.nonce) {
            return false;
        }
        if proof.verify(self.extension.origin(), path).await.is_err() {
            return false;
        }
        let key = self.extension.origin().store_key(path);
        if !(self.auth_only || self.store.contains(&key, &proof.address)) {
            return false;
        }
        self.store.record_nonce(&proof.nonce);
        true
    }
}

impl GateHooks for SiwxGate {
    fn on_protected_request<'a>(
        &'a self,
        req: &'a Request<Body>,
    ) -> impl Future<Output = ProtectedRequestOutcome> + Send + 'a {
        let header = req
            .headers()
            .get(SIGN_IN_WITH_X)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let path = req.uri().path().to_owned();
        async move {
            let Some(header) = header else {
                return ProtectedRequestOutcome::Continue;
            };
            if self.try_grant(&header, &path).await {
                ProtectedRequestOutcome::GrantAccess
            } else {
                ProtectedRequestOutcome::Continue
            }
        }
    }
}
