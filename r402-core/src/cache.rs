//! Shared cache primitives used by facilitator implementations.
//!
//! This module is only compiled with the `cache` feature flag. The types
//! are intentionally minimal wrappers over `moka::sync::Cache` tuned for
//! x402 use cases (bounded capacity, short TTL, `Send + Sync + Clone`).

#![cfg(feature = "cache")]
#![cfg_attr(docsrs, doc(cfg(feature = "cache")))]

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
/// Designed for **deduplication / idempotency** workloads: the `reserve`
/// method atomically inserts the key if absent, returning a [`Duplicate`]
/// discriminator. No value type is associated with each key — only presence
/// matters.
///
/// Backed by [`moka::sync::Cache`] internally; cloning shares the same
/// underlying storage thanks to moka's internal `Arc`.
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
    ///
    /// This operation is lock-free and coalesces concurrent inserts of the
    /// same key (exactly one insertion wins; others observe `Yes`).
    pub fn reserve(&self, key: impl Into<String>) -> Duplicate {
        let key = key.into();
        if self.inner.contains_key(&key) {
            return Duplicate::Yes;
        }
        // moka's `get_with_by_ref` coalesces concurrent callers; we use the
        // "entry and compute" API via `get_with` which is atomic under
        // moka's internal scheduler.
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
}

impl Debug for TtlSet {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("TtlSet")
            .field("entries", &self.inner.entry_count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
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
}
