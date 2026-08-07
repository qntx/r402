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

/// Spec-recommended TTL for cross-chain settlement deduplication.
///
/// Per x402 v2 §Duplicate Settlement Mitigation, facilitators SHOULD retain
/// a short-lived cache keyed by the unique settlement identifier (EIP-3009
/// nonce, Permit2 nonce, or base64 SVM transaction). The 2-minute window
/// covers:
///
/// - Solana blockhash lifetime (≈ 60 s) plus a safety multiplier of 2;
/// - Typical EVM finality (12 blocks × ≈ 12 s) plus margin so two requests
///   re-using the same EIP-3009 / Permit2 nonce are caught even when the
///   first settlement is mid-confirmation.
pub const DEFAULT_SETTLEMENT_TTL: Duration = Duration::from_mins(2);

/// Default upper bound on tracked settlement identifiers.
///
/// At ≈ 32 bytes per key (a 32-byte EIP-3009 nonce hex string) plus moka
/// overhead, the worst-case footprint is in the low MB range — small
/// enough that the bound mostly protects against unbounded growth from
/// adversarial replay storms rather than from steady-state traffic.
pub const DEFAULT_SETTLEMENT_CAPACITY: u64 = 10_000;

/// Chain-agnostic deduplication cache for facilitator settlement requests.
///
/// Wraps [`TtlSet`] with the canonical x402 v2 TTL ([`DEFAULT_SETTLEMENT_TTL`])
/// and capacity ([`DEFAULT_SETTLEMENT_CAPACITY`]). Used by every chain
/// crate to defend against duplicate / replayed `/settle` calls within
/// the on-chain replay window:
///
/// - **EVM exact** — keyed by `chain:nonce_hex` (the EIP-3009 nonce binds
///   one-shot to a single token + payer pair).
/// - **EVM upto** — keyed by `chain:permit2_nonce_hex`.
/// - **SVM exact** — keyed by the base64 transaction payload.
///
/// The cache is **in-memory only**. A facilitator deployment that spans
/// multiple processes must either pin requests to a single worker or
/// replace this cache with a Redis-backed implementation by wiring its
/// own [`TtlSet`]-equivalent through the same trait surface.
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

    /// Atomically reserves the key. Returns [`Duplicate::No`] when newly
    /// inserted (caller may proceed) or [`Duplicate::Yes`] when the key
    /// is already present (caller must abort with `DuplicateSettlement`).
    ///
    /// Increments the
    /// [`r402_settlement_cache_reserve_total`](crate::metrics::SETTLEMENT_CACHE_RESERVE_TOTAL)
    /// counter with `outcome=inserted|duplicate` when the `metrics`
    /// feature is enabled.
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
                crate::metrics::SETTLEMENT_CACHE_RESERVE_TOTAL,
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
}

impl Default for SettlementCache {
    fn default() -> Self {
        Self::new()
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

    #[test]
    fn settlement_cache_default_is_2_minute_ttl() {
        // Verify the spec-recommended TTL is wired correctly so future
        // refactors can't silently lengthen / shorten it.
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
}
