//! In-memory duplicate-settlement cache for NEAR exact settle.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use base64::Engine;
use sha2::{Digest, Sha256};

/// Safety-net TTL when neither [`SettlementCache::release`] nor
/// [`SettlementCache::evict_expired`] fires.
const DEFAULT_SETTLEMENT_TTL: Duration = Duration::from_mins(2);

#[derive(Debug)]
struct Entry {
    inserted_at: Instant,
    max_block_height: u64,
}

/// Deduplicates concurrent `/settle` calls for the same signed delegate.
///
/// Entries carry `max_block_height` so they can be dropped once the delegate
/// can no longer land. A TTL bounds memory if neither that nor `release` runs.
#[derive(Clone, Debug)]
pub struct SettlementCache {
    entries: Arc<Mutex<HashMap<String, Entry>>>,
    ttl: Duration,
}

impl Default for SettlementCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SettlementCache {
    /// Creates a cache with the 120s TTL safety net.
    #[must_use]
    pub fn new() -> Self {
        Self::with_ttl(DEFAULT_SETTLEMENT_TTL)
    }

    /// Creates a cache with a custom TTL safety net.
    #[must_use]
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            ttl,
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Entry>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Returns `true` if `key` is already in flight; otherwise records it.
    ///
    /// Callers MUST reject settlement when this returns `true`.
    #[must_use = "callers MUST reject settlement when this is true"]
    #[allow(
        clippy::significant_drop_tightening,
        reason = "mutex must stay held through insert"
    )]
    pub fn is_duplicate(&self, key: &str, max_block_height: u64) -> bool {
        let mut entries = self.lock();
        prune_by_ttl(&mut entries, self.ttl);
        if entries.contains_key(key) {
            return true;
        }
        entries.insert(
            key.to_owned(),
            Entry {
                inserted_at: Instant::now(),
                max_block_height,
            },
        );
        false
    }

    /// Drops `key` once the inner receipt outcome is known.
    pub fn release(&self, key: &str) {
        self.lock().remove(key);
    }

    /// Drops entries whose delegate window has passed (`max_block_height < current`).
    pub fn evict_expired(&self, current_block_height: u64) {
        self.lock()
            .retain(|_, entry| entry.max_block_height >= current_block_height);
    }
}

fn prune_by_ttl(entries: &mut HashMap<String, Entry>, ttl: Duration) {
    let now = Instant::now();
    entries.retain(|_, entry| now.saturating_duration_since(entry.inserted_at) < ttl);
}

/// Hex SHA-256 of the exact base64-decoded `signedDelegateAction` bytes.
#[must_use]
pub fn settlement_cache_key(signed_delegate_action_b64: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(signed_delegate_action_b64)
        .ok()?;
    Some(hex::encode(Sha256::digest(bytes)))
}
