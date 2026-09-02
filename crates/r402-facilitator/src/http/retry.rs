//! GET `/supported` retry policy: HTTP 429 only.

use std::time::{Duration, SystemTime};

use http::StatusCode;

/// Number of `/supported` attempts, matching official `GET_SUPPORTED_RETRIES`.
pub(super) const GET_SUPPORTED_RETRIES: u32 = 3;
/// Base delay in ms for exponential backoff when `Retry-After` is unusable.
const GET_SUPPORTED_RETRY_DELAY_MS: u64 = 1000;
/// Upper bound on retry delay.
const MAX_RETRY_DELAY_MS: u64 = 30_000;

/// Delay before the next `/supported` retry.
///
/// Parses `Retry-After` as RFC 7231 delta-seconds or HTTP-date. Missing,
/// invalid, or non-positive values fall back to `1000 * 2^attempt` ms.
/// The result is capped at 30 seconds.
#[must_use]
pub fn compute_retry_delay(retry_after: Option<&str>, attempt: u32) -> Duration {
    let parsed_ms = retry_after.and_then(parse_retry_after_ms);
    let delay_ms = match parsed_ms {
        Some(millis) if millis > 0 => millis,
        _ => GET_SUPPORTED_RETRY_DELAY_MS.saturating_mul(2u64.saturating_pow(attempt)),
    };
    Duration::from_millis(delay_ms.min(MAX_RETRY_DELAY_MS))
}

fn parse_retry_after_ms(raw: &str) -> Option<u64> {
    let trimmed = raw.trim();
    if !trimmed.is_empty() && trimmed.as_bytes().iter().all(u8::is_ascii_digit) {
        let seconds: u64 = trimmed.parse().ok()?;
        return Some(seconds.saturating_mul(1000));
    }
    let retry_at = httpdate::parse_http_date(trimmed).ok()?;
    retry_at
        .duration_since(SystemTime::now())
        .ok()
        .map(|delay| u64::try_from(delay.as_millis()).unwrap_or(u64::MAX))
}

/// Delay for another `/supported` attempt, if this status is retryable.
pub(super) fn supported_retry_delay(
    status: StatusCode,
    retry_after: Option<&str>,
    attempt: u32,
) -> Option<Duration> {
    if status == StatusCode::TOO_MANY_REQUESTS && attempt + 1 < GET_SUPPORTED_RETRIES {
        Some(compute_retry_delay(retry_after, attempt))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_retry_delay_delta_seconds() {
        assert_eq!(compute_retry_delay(Some("5"), 0), Duration::from_secs(5));
        assert_eq!(compute_retry_delay(Some("12"), 1), Duration::from_secs(12));
    }

    #[test]
    fn compute_retry_delay_http_date() {
        let future = SystemTime::now() + Duration::from_secs(7);
        let formatted = httpdate::fmt_http_date(future);
        let delay = compute_retry_delay(Some(&formatted), 0);
        assert!(
            delay > Duration::from_secs(5),
            "HTTP-date ~7s in the future, got {delay:?}"
        );
        assert!(
            delay <= Duration::from_secs(7),
            "HTTP-date ~7s in the future, got {delay:?}"
        );
    }

    #[test]
    fn compute_retry_delay_falls_back_to_exponential() {
        assert_eq!(compute_retry_delay(None, 0), Duration::from_secs(1));
        assert_eq!(compute_retry_delay(None, 1), Duration::from_secs(2));
        assert_eq!(compute_retry_delay(None, 2), Duration::from_secs(4));
        assert_eq!(compute_retry_delay(Some("0"), 0), Duration::from_secs(1));
        assert_eq!(compute_retry_delay(Some("-5"), 1), Duration::from_secs(2));
        assert_eq!(
            compute_retry_delay(Some("not-a-date"), 1),
            Duration::from_secs(2)
        );
        assert_eq!(compute_retry_delay(Some("1.5"), 1), Duration::from_secs(2));
        assert_eq!(
            compute_retry_delay(Some("9999"), 0),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn supported_retry_delay_only_429_and_remaining_attempts() {
        assert!(supported_retry_delay(StatusCode::TOO_MANY_REQUESTS, None, 0).is_some());
        assert!(supported_retry_delay(StatusCode::TOO_MANY_REQUESTS, None, 1).is_some());
        assert!(supported_retry_delay(StatusCode::TOO_MANY_REQUESTS, None, 2).is_none());
        assert!(supported_retry_delay(StatusCode::INTERNAL_SERVER_ERROR, None, 0).is_none());
    }
}
