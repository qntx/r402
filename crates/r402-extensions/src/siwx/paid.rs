//! In-memory store of paid addresses and used nonces.
//!
//! Key is configured origin + request path. Only
//! [`SettleResponse::Success`](r402_protocol::payment::SettleResponse) is
//! recorded. v1 has no Redis and no TTL eviction.
//!
//! [`PaidAddressStore::consume_nonce`] is insert-if-absent under one lock.
//! HTTP consumes before CAIP-122 verify. A failed verify does not restore
//! the nonce.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// Addresses that completed a successful settlement, plus consumed nonces.
pub trait PaidAddressStore: Send + Sync {
    /// Whether `address` already paid for `store_key`.
    fn contains(&self, store_key: &str, address: &str) -> bool;

    /// Records a successful settlement. Failures and verifies are ignored.
    fn record_success(&self, store_key: &str, address: &str);

    /// Whether `nonce` is already in the used set.
    fn has_used_nonce(&self, nonce: &str) -> bool;

    /// Marks `nonce` as used. Idempotent.
    fn record_nonce(&self, nonce: &str);

    /// Inserts `nonce` if absent. `true` means this caller consumed it.
    ///
    /// Must be atomic. A later failed verify must not restore the nonce.
    #[must_use]
    fn consume_nonce(&self, nonce: &str) -> bool;
}

#[derive(Debug, Default)]
struct Inner {
    /// `store_key` → paid addresses (EVM addresses stored lowercase).
    paid: HashMap<String, HashSet<String>>,
    nonces: HashSet<String>,
}

/// Process-local [`PaidAddressStore`].
///
/// [`Clone`] shares the map so the HTTP gate and settle-success path see
/// the same addresses and nonces.
#[derive(Debug, Clone, Default)]
pub struct InMemoryPaidAddressStore {
    inner: Arc<Mutex<Inner>>,
}

impl InMemoryPaidAddressStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl PaidAddressStore for InMemoryPaidAddressStore {
    fn contains(&self, store_key: &str, address: &str) -> bool {
        let key = normalize_address(address);
        self.lock()
            .paid
            .get(store_key)
            .is_some_and(|set| set.contains(&key))
    }

    fn record_success(&self, store_key: &str, address: &str) {
        let key = normalize_address(address);
        let mut inner = self.lock();
        inner
            .paid
            .entry(store_key.to_owned())
            .or_default()
            .insert(key);
    }

    fn has_used_nonce(&self, nonce: &str) -> bool {
        self.lock().nonces.contains(nonce)
    }

    fn record_nonce(&self, nonce: &str) {
        let _ = self.lock().nonces.insert(nonce.to_owned());
    }

    fn consume_nonce(&self, nonce: &str) -> bool {
        self.lock().nonces.insert(nonce.to_owned())
    }
}

/// EVM `0x` addresses compare case-insensitively. Solana base58 is left as-is.
fn normalize_address(address: &str) -> String {
    if address
        .get(..2)
        .is_some_and(|p| p.eq_ignore_ascii_case("0x"))
    {
        address.to_ascii_lowercase()
    } else {
        address.to_owned()
    }
}
