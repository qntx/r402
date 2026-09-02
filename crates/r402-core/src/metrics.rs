//! Stable metric names emitted by the r402 stack.
//!
//! r402 instruments hot paths through the [`metrics`] facade so any
//! recorder (Prometheus, `StatsD`, OpenTelemetry, ...) can ingest the
//! counters and histograms without coupling the SDK to a specific
//! exporter. Operators install a recorder at startup; with no recorder
//! the macros expand to no-ops.
//!
//! # Naming
//!
//! All metrics use the `r402_` prefix and follow the [Prometheus naming
//! conventions]: `snake_case`, base-unit suffixes (`_seconds`, `_total`),
//! and a `_total` suffix for monotonically increasing counters.
//!
//! [`metrics`]: https://docs.rs/metrics/0.24
//! [Prometheus naming conventions]: https://prometheus.io/docs/practices/naming/
//!
//! # Cardinality
//!
//! Labels are bounded: `result` is one of three values per metric, and
//! we never include payer addresses, payment IDs, or signatures. This
//! keeps the cardinality flat and the recorder memory bounded even
//! under adversarial load.

#![cfg(feature = "metrics")]
#![cfg_attr(docsrs, doc(cfg(feature = "metrics")))]

use std::time::Duration;

use crate::error::FacilitatorError;
use crate::wire::{SettleResponse, VerifyResponse};

/// Counter incremented every time a facilitator's `verify` returns.
///
/// Recorded by [`crate::ResourceServer`] on the inner facilitator call.
///
/// Labels:
/// - `result` — `valid` for `VerifyResponse::Valid`, `invalid` for
///   `VerifyResponse::Invalid`, or `error` when the call returned a
///   structured `FacilitatorError`.
pub const FACILITATOR_VERIFY_TOTAL: &str = "r402_facilitator_verify_total";

/// Counter incremented every time a facilitator's `settle` returns.
///
/// Recorded by [`crate::ResourceServer`] on the inner facilitator call.
///
/// Labels:
/// - `result` — `success` for `SettleResponse::Success`, `failure` for
///   `SettleResponse::Failure`, or `error` for transport / RPC errors.
pub const FACILITATOR_SETTLE_TOTAL: &str = "r402_facilitator_settle_total";

/// Histogram of facilitator `verify` durations in seconds.
///
/// Recorded by [`crate::ResourceServer`] on the inner facilitator call.
pub const FACILITATOR_VERIFY_DURATION_SECONDS: &str = "r402_facilitator_verify_duration_seconds";

/// Histogram of facilitator `settle` durations in seconds.
///
/// Recorded by [`crate::ResourceServer`] on the inner facilitator call.
pub const FACILITATOR_SETTLE_DURATION_SECONDS: &str = "r402_facilitator_settle_duration_seconds";

/// Counter incremented every time the settlement deduplication cache
/// observes a key.
///
/// Labels:
/// - `outcome` — `inserted` for newly seen keys, `duplicate` for hits.
pub const SETTLEMENT_CACHE_RESERVE_TOTAL: &str = "r402_settlement_cache_reserve_total";

/// Counter incremented when the background settlement supervisor records
/// an outcome.
///
/// Labels:
/// - `result` — `ok`, `error`, `panic`, or `cancelled`.
pub const PAYGATE_BACKGROUND_SETTLE_TOTAL: &str = "r402_paygate_background_settle_total";

/// Emits verify counter + duration for one inner facilitator call.
pub(crate) fn record_verify(result: &Result<VerifyResponse, FacilitatorError>, elapsed: Duration) {
    let label = match result {
        Ok(VerifyResponse::Valid { .. }) => "valid",
        Ok(VerifyResponse::Invalid { .. }) => "invalid",
        Err(_) => "error",
    };
    ::metrics::counter!(FACILITATOR_VERIFY_TOTAL, "result" => label).increment(1);
    ::metrics::histogram!(FACILITATOR_VERIFY_DURATION_SECONDS).record(elapsed);
}

/// Emits settle counter + duration for one inner facilitator call.
pub(crate) fn record_settle(result: &Result<SettleResponse, FacilitatorError>, elapsed: Duration) {
    let label = match result {
        Ok(SettleResponse::Success { .. }) => "success",
        Ok(SettleResponse::Failure { .. }) => "failure",
        Err(_) => "error",
    };
    ::metrics::counter!(FACILITATOR_SETTLE_TOTAL, "result" => label).increment(1);
    ::metrics::histogram!(FACILITATOR_SETTLE_DURATION_SECONDS).record(elapsed);
}
