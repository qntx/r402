//! Facilitator-side store for a broadcast-but-unconfirmed settlement hash.
//!
//! A [`crate::facilitator::Facilitator::settle`] implementation MUST NOT return
//! [`crate::error::ErrorReason::SettlementPending`] unless it writes the
//! broadcast hash here on the first attempt and reads it on the retry to
//! reconcile instead of broadcasting again.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use compact_str::CompactString;

/// TTL applied by [`InMemoryPendingSettlementStore`]. Other store
/// implementations choose their own expiry.
pub const PENDING_SETTLEMENT_TTL: Duration = Duration::from_mins(5);

/// Remembers a broadcast transaction hash keyed by a payload-derived id
/// (EIP-3009/Permit2 signature, SVM message hash, ...).
///
/// Implementations must be safe for concurrent use. Multi-instance
/// facilitators inject a shared backend; the in-memory default only
/// works when the retry lands on the same process.
pub trait PendingSettlementStore: Send + Sync {
    /// Previously stored hash for `key`, if any. Expired entries are absent.
    fn get(&self, key: &str) -> Option<CompactString>;

    /// Records that `key` broadcast `tx_hash` and is not yet confirmed.
    /// A later `set` for the same key overwrites.
    fn set(&self, key: &str, tx_hash: CompactString);

    /// Drops any pending entry for `key` (confirmed success or terminal failure).
    fn delete(&self, key: &str);
}

#[derive(Debug)]
struct PendingSettlementEntry {
    tx_hash: CompactString,
    stored_at: Instant,
}

/// Process-local [`PendingSettlementStore`] with lazy TTL pruning on [`get`](Self::get).
#[derive(Debug)]
pub struct InMemoryPendingSettlementStore {
    entries: Mutex<HashMap<String, PendingSettlementEntry>>,
}

impl Default for InMemoryPendingSettlementStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryPendingSettlementStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, PendingSettlementEntry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn prune(entries: &mut HashMap<String, PendingSettlementEntry>) {
        entries.retain(|_, entry| entry.stored_at.elapsed() < PENDING_SETTLEMENT_TTL);
    }

    /// Snapshot of key → hash. Test-only.
    #[cfg(test)]
    #[must_use]
    pub fn entries_snapshot(&self) -> HashMap<String, CompactString> {
        self.lock()
            .iter()
            .map(|(k, v)| (k.clone(), v.tx_hash.clone()))
            .collect()
    }

    /// Moves `key`'s stored-at timestamp back by `age`. Test-only.
    #[cfg(test)]
    pub fn backdate(&self, key: &str, age: Duration) {
        let mut entries = self.lock();
        if let Some(entry) = entries.get_mut(key)
            && let Some(stored_at) = Instant::now().checked_sub(age)
        {
            entry.stored_at = stored_at;
        }
    }
}

impl PendingSettlementStore for InMemoryPendingSettlementStore {
    fn get(&self, key: &str) -> Option<CompactString> {
        let mut entries = self.lock();
        Self::prune(&mut entries);
        entries.get(key).map(|entry| entry.tx_hash.clone())
    }

    fn set(&self, key: &str, tx_hash: CompactString) {
        self.lock().insert(
            key.to_owned(),
            PendingSettlementEntry {
                tx_hash,
                stored_at: Instant::now(),
            },
        );
    }

    fn delete(&self, key: &str) {
        self.lock().remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_miss() {
        let store = InMemoryPendingSettlementStore::new();
        assert!(store.get("missing").is_none());
    }

    #[test]
    fn set_then_get() {
        let store = InMemoryPendingSettlementStore::new();
        store.set("k", "0xabc".into());
        assert_eq!(store.get("k").as_deref(), Some("0xabc"));
    }

    #[test]
    fn set_overwrites() {
        let store = InMemoryPendingSettlementStore::new();
        store.set("k", "0xold".into());
        store.set("k", "0xnew".into());
        assert_eq!(store.get("k").as_deref(), Some("0xnew"));
    }

    #[test]
    fn delete_removes() {
        let store = InMemoryPendingSettlementStore::new();
        store.set("k", "0xabc".into());
        store.delete("k");
        assert!(store.get("k").is_none());
    }

    #[test]
    fn delete_missing_is_noop() {
        let store = InMemoryPendingSettlementStore::new();
        store.delete("missing");
        assert!(store.get("missing").is_none());
    }

    #[test]
    fn distinct_keys_do_not_collide() {
        let store = InMemoryPendingSettlementStore::new();
        store.set("a", "0xa".into());
        store.set("b", "0xb".into());
        assert_eq!(store.get("a").as_deref(), Some("0xa"));
        assert_eq!(store.get("b").as_deref(), Some("0xb"));
    }

    #[test]
    fn expired_entries_pruned_on_get() {
        let store = InMemoryPendingSettlementStore::new();
        store.set("k", "0xabc".into());
        store.backdate("k", PENDING_SETTLEMENT_TTL + Duration::from_secs(1));
        assert!(store.get("k").is_none());
        assert!(store.entries_snapshot().is_empty());
    }

    #[test]
    fn fresh_entries_survive_pruning() {
        let store = InMemoryPendingSettlementStore::new();
        store.set("fresh", "0xf".into());
        store.set("stale", "0xs".into());
        store.backdate("stale", PENDING_SETTLEMENT_TTL + Duration::from_secs(1));
        assert_eq!(store.get("fresh").as_deref(), Some("0xf"));
        assert!(store.get("stale").is_none());
    }

    #[test]
    fn set_refreshes_timestamp() {
        let store = InMemoryPendingSettlementStore::new();
        store.set("k", "0xold".into());
        store.backdate("k", PENDING_SETTLEMENT_TTL + Duration::from_secs(1));
        store.set("k", "0xnew".into());
        assert_eq!(store.get("k").as_deref(), Some("0xnew"));
    }

    #[test]
    fn trait_object_roundtrip() {
        let store: &dyn PendingSettlementStore = &InMemoryPendingSettlementStore::new();
        store.set("k", "0xabc".into());
        assert_eq!(store.get("k").as_deref(), Some("0xabc"));
        store.delete("k");
        assert!(store.get("k").is_none());
    }
}
