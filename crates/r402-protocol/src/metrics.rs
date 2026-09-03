//! Stable metric names emitted by the r402 stack.
//!
//! Names use the `r402_` prefix and Prometheus suffixes (`_total`,
//! `_seconds`). Labels stay low-cardinality: never payer addresses, payment
//! IDs, or signatures.

/// Facilitator `verify` outcomes.
///
/// Labels: `result` = `valid` | `invalid` | `error`.
pub const FACILITATOR_VERIFY_TOTAL: &str = "r402_facilitator_verify_total";

/// Facilitator `settle` outcomes.
///
/// Labels: `result` = `success` | `failure` | `error`.
pub const FACILITATOR_SETTLE_TOTAL: &str = "r402_facilitator_settle_total";

/// Histogram of facilitator `verify` duration in seconds.
pub const FACILITATOR_VERIFY_DURATION_SECONDS: &str = "r402_facilitator_verify_duration_seconds";

/// Histogram of facilitator `settle` duration in seconds.
pub const FACILITATOR_SETTLE_DURATION_SECONDS: &str = "r402_facilitator_settle_duration_seconds";

/// Settlement-dedup cache observations.
///
/// Labels: `outcome` = `inserted` | `duplicate`.
pub const SETTLEMENT_CACHE_RESERVE_TOTAL: &str = "r402_settlement_cache_reserve_total";

/// Settlement-dedup cache releases (terminal failure without a broadcast hash).
pub const SETTLEMENT_CACHE_RELEASE_TOTAL: &str = "r402_settlement_cache_release_total";

/// Background settlement supervisor outcomes.
///
/// Labels: `result` = `ok` | `error` | `panic` | `cancelled`.
pub const BACKGROUND_SETTLE_TOTAL: &str = "r402_background_settle_total";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_stable() {
        assert_eq!(FACILITATOR_VERIFY_TOTAL, "r402_facilitator_verify_total");
        assert_eq!(FACILITATOR_SETTLE_TOTAL, "r402_facilitator_settle_total");
        assert_eq!(
            FACILITATOR_VERIFY_DURATION_SECONDS,
            "r402_facilitator_verify_duration_seconds"
        );
        assert_eq!(
            FACILITATOR_SETTLE_DURATION_SECONDS,
            "r402_facilitator_settle_duration_seconds"
        );
        assert_eq!(
            SETTLEMENT_CACHE_RESERVE_TOTAL,
            "r402_settlement_cache_reserve_total"
        );
        assert_eq!(
            SETTLEMENT_CACHE_RELEASE_TOTAL,
            "r402_settlement_cache_release_total"
        );
        assert_eq!(BACKGROUND_SETTLE_TOTAL, "r402_background_settle_total");
        assert!(
            !BACKGROUND_SETTLE_TOTAL.contains("paygate"),
            "metric names must not contain paygate"
        );
    }
}
