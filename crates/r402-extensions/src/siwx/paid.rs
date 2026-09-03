//! In-memory store of addresses that have already paid.
//!
//! Key is configured origin + request path. Only
//! [`SettleResponse::Success`](r402_protocol::payment::SettleResponse) is
//! recorded. v1 has no Redis and no TTL eviction.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// Addresses that completed a successful settlement for a resource key.
pub trait PaidAddressStore: Send + Sync {
    /// Whether `address` already paid for `store_key`.
    fn contains(&self, store_key: &str, address: &str) -> bool;

    /// Records a successful settlement. Failures and verifies are ignored.
    fn record_success(&self, store_key: &str, address: &str);
}

/// Process-local [`PaidAddressStore`].
///
/// [`Clone`] shares the map so the HTTP gate and settle-success path see
/// the same addresses.
#[derive(Debug, Clone, Default)]
pub struct InMemoryPaidAddressStore {
    inner: Arc<Mutex<HashMap<String, HashSet<String>>>>,
}

impl InMemoryPaidAddressStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, HashSet<String>>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl PaidAddressStore for InMemoryPaidAddressStore {
    fn contains(&self, store_key: &str, address: &str) -> bool {
        self.lock()
            .get(store_key)
            .is_some_and(|set| set.contains(address))
    }

    fn record_success(&self, store_key: &str, address: &str) {
        let mut map = self.lock();
        map.entry(store_key.to_owned())
            .or_default()
            .insert(address.to_owned());
    }
}
