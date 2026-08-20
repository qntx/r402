//! Facilitator settlement for the XRPL exact scheme.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use compact_str::CompactString;
use r402_core::cache::{DEFAULT_SETTLEMENT_CAPACITY, Duplicate};
use r402_core::error::ErrorReason;
use r402_core::wire::{Extensions, SettleResponse, VerifyResponse};
use serde_json::Value;

use super::verify::verify_request_json;
use crate::SETTLEMENT_TTL_MS;
use crate::chain::codec::{decode_signed_tx_blob, signed_tx_hash};
use crate::chain::rpc::{XrplRpc, XrplRpcError, XrplTxResult};

/// Per-entry settlement dedup keyed by transaction hash.
///
/// TTL is the payment's landable window (`maxTimeoutSeconds`) plus the 120s
/// floor, matching the scheme's `LastLedgerSequence` policy. A global-TTL
/// cache would evict while the blob is still landable.
#[derive(Clone, Debug)]
pub struct XrplSettlementCache {
    entries: Arc<Mutex<HashMap<String, Instant>>>,
}

impl Default for XrplSettlementCache {
    fn default() -> Self {
        Self::new()
    }
}

impl XrplSettlementCache {
    /// Creates an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Reserves `key` for `ttl`.
    ///
    /// Returns [`Duplicate::Yes`] when already present. Returns `None` when
    /// the cache is at [`DEFAULT_SETTLEMENT_CAPACITY`] and the key is new
    /// (fail-closed; does not evict landable hashes).
    #[must_use = "callers MUST honour the Duplicate outcome to enforce idempotency"]
    #[allow(
        clippy::significant_drop_tightening,
        reason = "mutex must stay held through insert"
    )]
    pub fn reserve(&self, key: impl Into<String>, ttl: Duration) -> Option<Duplicate> {
        let now = Instant::now();
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.retain(|_, expires_at| *expires_at > now);
        let key = key.into();
        if entries.contains_key(&key) {
            return Some(Duplicate::Yes);
        }
        if entries.len() >= usize::try_from(DEFAULT_SETTLEMENT_CAPACITY).unwrap_or(10_000) {
            return None;
        }
        entries.insert(key, now + ttl);
        Some(Duplicate::No)
    }
}

/// Landable-window TTL: `maxTimeoutSeconds` plus the 120s floor.
#[must_use]
pub fn settlement_ttl(max_timeout_seconds: u64) -> Duration {
    let landable_ms = max_timeout_seconds.saturating_mul(1000);
    Duration::from_millis(
        landable_ms
            .saturating_add(SETTLEMENT_TTL_MS)
            .max(SETTLEMENT_TTL_MS),
    )
}

/// Settles a verified XRPL payment by submitting the signed blob.
pub async fn settle_request<R, F, Fut>(
    rpc: &R,
    cache: &XrplSettlementCache,
    max_fee_drops: u64,
    request: &Value,
    submit: F,
) -> SettleResponse
where
    R: XrplRpc,
    F: FnOnce(String, u32) -> Fut,
    Fut: Future<Output = Result<XrplTxResult, XrplRpcError>> + Send,
{
    let network = request
        .get("paymentRequirements")
        .and_then(|r| r.get("network"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();

    let verified = verify_request_json(rpc, max_fee_drops, request).await;
    let payer = match verified {
        VerifyResponse::Valid { payer, .. } => Some(payer),
        VerifyResponse::Invalid { payer, reason, .. } => {
            return settle_failure(reason, &network, payer);
        }
        _ => {
            return settle_failure(
                ErrorReason::from_wire("unexpected_verify_error"),
                &network,
                None,
            );
        }
    };

    let signed_blob = request
        .get("paymentPayload")
        .and_then(|p| p.get("payload"))
        .and_then(|p| p.get("signedTxBlob"))
        .and_then(Value::as_str);
    let Some(signed_blob) = signed_blob else {
        return settle_failure(
            ErrorReason::from_wire("invalid_exact_xrpl_payload_shape"),
            &network,
            payer,
        );
    };

    let Ok(hash) = signed_tx_hash(signed_blob) else {
        return settle_failure(
            ErrorReason::from_wire("invalid_exact_xrpl_payload_shape"),
            &network,
            payer,
        );
    };

    let last_ledger = decode_signed_tx_blob(signed_blob)
        .ok()
        .and_then(|tx| tx.get("LastLedgerSequence").and_then(Value::as_u64))
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0);

    let max_timeout = request
        .get("paymentRequirements")
        .and_then(|r| r.get("maxTimeoutSeconds"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    match cache.reserve(&hash, settlement_ttl(max_timeout)) {
        Some(Duplicate::Yes) => {
            return settle_failure(ErrorReason::DuplicateSettlement, &network, payer);
        }
        None => {
            return settle_failure(
                ErrorReason::from_wire("invalid_exact_xrpl_facilitator_error"),
                &network,
                payer,
            );
        }
        Some(Duplicate::No) => {}
    }

    match submit(signed_blob.to_owned(), last_ledger).await {
        Ok(outcome) if outcome.validated && outcome.result_code == "tesSUCCESS" => {
            SettleResponse::Success {
                payer: payer.unwrap_or_default(),
                transaction: outcome.hash.into(),
                network: network.into(),
                amount: request
                    .get("paymentRequirements")
                    .and_then(|r| r.get("amount"))
                    .and_then(Value::as_str)
                    .map(CompactString::from),
                extensions: Extensions::new(),
            }
        }
        Ok(outcome) => SettleResponse::Failure {
            reason: ErrorReason::from_wire(&format!("transaction_failed: {}", outcome.result_code)),
            message: None,
            payer,
            network: network.into(),
            extensions: Extensions::new(),
        },
        Err(err) => SettleResponse::Failure {
            reason: ErrorReason::from_wire(&format!("transaction_failed: {err}")),
            message: Some(err.to_string().into()),
            payer,
            network: network.into(),
            extensions: Extensions::new(),
        },
    }
}

fn settle_failure(
    reason: ErrorReason,
    network: &str,
    payer: Option<CompactString>,
) -> SettleResponse {
    SettleResponse::Failure {
        reason,
        message: None,
        payer,
        network: network.into(),
        extensions: Extensions::new(),
    }
}

#[cfg(test)]
mod ttl_tests {
    use super::*;

    #[test]
    fn ttl_covers_timeout_and_floors_at_120s() {
        assert_eq!(settlement_ttl(0), Duration::from_millis(SETTLEMENT_TTL_MS));
        assert_eq!(
            settlement_ttl(60),
            Duration::from_millis(60_000 + SETTLEMENT_TTL_MS)
        );
        assert_eq!(
            settlement_ttl(300),
            Duration::from_millis(300_000 + SETTLEMENT_TTL_MS)
        );
    }
}
