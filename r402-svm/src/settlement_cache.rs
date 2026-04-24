//! TTL-bounded deduplication cache for Solana settlement requests.
//!
//! Per x402 v2 §Duplicate Settlement Mitigation, facilitators SHOULD retain
//! a short-lived cache keyed by the exact transaction payload so that an
//! attacker cannot submit the same transaction twice and cause double
//! settlement (the underlying RPC can cache recent signatures on the hot
//! path, but that protection vanishes quickly). The cache TTL should be at
//! least 2× a Solana blockhash lifetime (~120 s).
//!
//! The cache is **in-memory only**. A production facilitator that spans
//! multiple processes must either pin requests to a single worker or
//! replace this cache with a Redis-backed implementation.

#![cfg(feature = "facilitator")]

use std::time::Duration;

use r402_core::cache::TtlSet;
pub use r402_core::cache::Duplicate;

/// Recommended cache TTL: 2× Solana blockhash lifetime (~60 s → 120 s).
pub const SETTLEMENT_TTL: Duration = Duration::from_secs(120);

/// Default upper bound on distinct tracked payloads to protect against
/// memory exhaustion.
pub const SETTLEMENT_CAPACITY: u64 = 10_000;

/// Cache for deduplicating concurrent or replayed settle attempts by
/// base64-encoded transaction payload.
#[derive(Debug, Clone)]
pub struct SettlementCache {
    inner: TtlSet,
}

impl SettlementCache {
    /// Creates a cache with the canonical TTL and capacity.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: TtlSet::new(SETTLEMENT_TTL, SETTLEMENT_CAPACITY),
        }
    }

    /// Creates a cache with custom parameters.
    #[must_use]
    pub fn with_params(ttl: Duration, capacity: u64) -> Self {
        Self {
            inner: TtlSet::new(ttl, capacity),
        }
    }

    /// Atomically attempts to reserve the key. Returns [`Duplicate::Yes`]
    /// if it was already present, [`Duplicate::No`] otherwise.
    #[must_use = "the caller must honour the Duplicate outcome"]
    pub fn reserve(&self, key: &str) -> Duplicate {
        self.inner.reserve(key)
    }

    /// Returns the current approximate entry count.
    #[must_use]
    pub fn entry_count(&self) -> u64 {
        self.inner.entry_count()
    }
}

impl Default for SettlementCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserves_once_then_yields_duplicate() {
        let cache = SettlementCache::new();
        assert_eq!(cache.reserve("tx-abc"), Duplicate::No);
        assert_eq!(cache.reserve("tx-abc"), Duplicate::Yes);
    }

    #[test]
    fn independent_keys_do_not_collide() {
        let cache = SettlementCache::new();
        assert_eq!(cache.reserve("a"), Duplicate::No);
        assert_eq!(cache.reserve("b"), Duplicate::No);
        assert_eq!(cache.reserve("a"), Duplicate::Yes);
        assert_eq!(cache.reserve("b"), Duplicate::Yes);
    }
}
