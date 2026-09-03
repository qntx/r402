//! Deduplication cache for facilitator settlement identifiers.

use std::fmt::{self, Debug, Formatter};
use std::time::Duration;

use moka::sync::Cache;

/// Outcome of a cache reservation call.
///
/// Returned by [`TtlSet::reserve`] to signal whether the key was newly
/// inserted (`No`, caller may proceed) or already present (`Yes`, caller
/// must abort).
#[must_use]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Duplicate {
    /// Key already present in the cache; caller should reject the request.
    Yes,
    /// Key was newly inserted; caller may proceed.
    No,
}

/// A Send+Sync cache of string keys with bounded capacity and TTL expiry.
///
/// Designed for **deduplication / idempotency** workloads: [`TtlSet::reserve`]
/// atomically inserts the key if absent. No value type is associated with
/// each key — only presence matters.
#[derive(Clone)]
pub struct TtlSet {
    inner: Cache<String, ()>,
}

impl TtlSet {
    /// Constructs a cache with the given TTL and maximum capacity.
    #[must_use]
    pub fn new(ttl: Duration, max_capacity: u64) -> Self {
        let inner = Cache::builder()
            .time_to_live(ttl)
            .max_capacity(max_capacity)
            .build();
        Self { inner }
    }

    /// Reserves a key: returns [`Duplicate::No`] if newly inserted,
    /// [`Duplicate::Yes`] if already present.
    pub fn reserve(&self, key: impl Into<String>) -> Duplicate {
        let key = key.into();
        if self.inner.contains_key(&key) {
            return Duplicate::Yes;
        }
        let mut was_new = false;
        let () = self.inner.get_with(key, || {
            was_new = true;
        });
        if was_new {
            Duplicate::No
        } else {
            Duplicate::Yes
        }
    }

    /// Returns `true` if `key` is currently present in the cache.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.inner.contains_key(key)
    }

    /// Returns the approximate current entry count (used for observability).
    #[must_use]
    pub fn entry_count(&self) -> u64 {
        self.inner.entry_count()
    }

    /// Drops `key` so a later [`Self::reserve`] can succeed.
    pub fn release(&self, key: &str) {
        self.inner.invalidate(key);
    }
}

impl Debug for TtlSet {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("TtlSet")
            .field("entries", &self.inner.entry_count())
            .finish()
    }
}

/// Two-minute window covers SVM blockhash lifetime and typical EVM finality.
pub const DEFAULT_SETTLEMENT_TTL: Duration = Duration::from_mins(2);

/// Caps replay-storm growth; steady-state footprint stays in the low MB range.
pub const DEFAULT_SETTLEMENT_CAPACITY: u64 = 10_000;

/// Chain-agnostic deduplication cache for facilitator settlement requests.
///
/// In-memory only. Multi-process deployments must pin requests to one worker
/// or inject a shared backend.
#[derive(Debug, Clone)]
pub struct SettlementCache {
    inner: TtlSet,
}

impl SettlementCache {
    /// Constructs a cache with the spec-recommended TTL and capacity.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: TtlSet::new(DEFAULT_SETTLEMENT_TTL, DEFAULT_SETTLEMENT_CAPACITY),
        }
    }

    /// Constructs a cache with custom TTL and capacity.
    #[must_use]
    pub fn with_params(ttl: Duration, capacity: u64) -> Self {
        Self {
            inner: TtlSet::new(ttl, capacity),
        }
    }

    /// Atomically reserves the key. Callers MUST honour [`Duplicate`].
    #[must_use = "callers MUST honour the Duplicate outcome to enforce idempotency"]
    pub fn reserve(&self, key: impl Into<String>) -> Duplicate {
        let outcome = self.inner.reserve(key);
        #[cfg(feature = "metrics")]
        {
            let label = match outcome {
                Duplicate::No => "inserted",
                Duplicate::Yes => "duplicate",
            };
            ::metrics::counter!(
                r402_protocol::metrics::SETTLEMENT_CACHE_RESERVE_TOTAL,
                "outcome" => label,
            )
            .increment(1);
        }
        outcome
    }

    /// Returns the approximate current entry count (observability hook).
    #[must_use]
    pub fn entry_count(&self) -> u64 {
        self.inner.entry_count()
    }

    /// Drops a previously reserved key (terminal failure with no broadcast hash).
    pub fn release(&self, key: &str) {
        self.inner.release(key);
        #[cfg(feature = "metrics")]
        {
            ::metrics::counter!(r402_protocol::metrics::SETTLEMENT_CACHE_RELEASE_TOTAL)
                .increment(1);
        }
    }
}

impl Default for SettlementCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use wiremock as _;

    use super::*;

    fn cache() -> TtlSet {
        TtlSet::new(Duration::from_mins(1), 1024)
    }

    #[test]
    fn reserve_fresh_key_is_new() {
        let cache = cache();
        assert_eq!(cache.reserve("a"), Duplicate::No);
    }

    #[test]
    fn reserve_same_key_twice_is_duplicate() {
        let cache = cache();
        assert_eq!(cache.reserve("a"), Duplicate::No);
        assert_eq!(cache.reserve("a"), Duplicate::Yes);
    }

    #[test]
    fn distinct_keys_are_independent() {
        let cache = cache();
        assert_eq!(cache.reserve("a"), Duplicate::No);
        assert_eq!(cache.reserve("b"), Duplicate::No);
        assert_eq!(cache.reserve("a"), Duplicate::Yes);
        assert_eq!(cache.reserve("b"), Duplicate::Yes);
    }

    #[test]
    fn settlement_cache_default_is_2_minute_ttl() {
        assert_eq!(DEFAULT_SETTLEMENT_TTL, Duration::from_mins(2));
    }

    #[test]
    fn settlement_cache_reserves_then_dedups() {
        let cache = SettlementCache::new();
        assert_eq!(cache.reserve("0xabc"), Duplicate::No);
        assert_eq!(cache.reserve("0xabc"), Duplicate::Yes);
    }

    #[test]
    fn settlement_cache_independent_keys() {
        let cache = SettlementCache::new();
        assert_eq!(cache.reserve("eip155:8453:0xnonce_a"), Duplicate::No);
        assert_eq!(cache.reserve("eip155:8453:0xnonce_b"), Duplicate::No);
        assert_eq!(cache.reserve("eip155:8453:0xnonce_a"), Duplicate::Yes);
        assert_eq!(cache.reserve("eip155:8453:0xnonce_b"), Duplicate::Yes);
    }

    #[test]
    fn settlement_cache_release_allows_reserve_again() {
        let cache = SettlementCache::new();
        assert_eq!(cache.reserve("k"), Duplicate::No);
        assert_eq!(cache.reserve("k"), Duplicate::Yes);
        cache.release("k");
        assert_eq!(cache.reserve("k"), Duplicate::No);
    }

    #[test]
    fn settlement_cache_release_missing_is_noop() {
        let cache = SettlementCache::new();
        cache.release("missing");
        assert_eq!(cache.reserve("missing"), Duplicate::No);
    }
}
