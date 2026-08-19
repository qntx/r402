//! JSON-RPC + Horizon client for Stellar.

use std::future::Future;
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use stellar_rpc_client::{
    Client, GetTransactionResponse, SendTransactionResponse, SimulateTransactionResponse,
};
use stellar_xdr::{AccountEntry, Hash, Limits, TransactionEnvelope, WriteXdr};

use super::types::{StellarChainReference, StellarRpcUrlError};
use crate::{DEFAULT_ESTIMATED_LEDGER_SECONDS, HORIZON_LEDGERS_SAMPLE_SIZE};

/// Errors from Stellar RPC or Horizon.
#[derive(Debug, thiserror::Error)]
pub enum StellarRpcError {
    /// Operator must supply a pubnet RPC URL.
    #[error(transparent)]
    RpcUrl(#[from] StellarRpcUrlError),
    /// Official `stellar-rpc-client` error, preserved 1:1.
    #[error(transparent)]
    Client(Box<stellar_rpc_client::Error>),
    /// Horizon request failed; callers should fall back to the default estimate.
    #[error("stellar horizon error: {0}")]
    Horizon(String),
    /// Local JSON-RPC / XDR request failed before a typed client error.
    #[error("stellar rpc request failed: {0}")]
    Request(String),
    /// `sendTransaction` returned a status other than `PENDING` / `DUPLICATE`.
    #[error("stellar sendTransaction status {status} (hash={hash:?}): {detail}")]
    SendRejected {
        /// RPC `status` field (`ERROR`, `TRY_AGAIN_LATER`, …).
        status: String,
        /// Transaction hash when the RPC included one.
        hash: Option<String>,
        /// `errorResultXdr` or a short status note.
        detail: String,
    },
    /// Poll observed on-chain `FAILED`.
    #[error("stellar transaction failed (hash={hash}): {detail}")]
    TransactionFailed {
        /// Submitted transaction hash.
        hash: String,
        /// SDK / result detail.
        detail: String,
    },
    /// Poll timed out before `SUCCESS` / `FAILED`.
    #[error("stellar transaction timeout (hash={hash})")]
    TransactionTimeout {
        /// Submitted transaction hash.
        hash: String,
    },
}

impl From<stellar_rpc_client::Error> for StellarRpcError {
    fn from(value: stellar_rpc_client::Error) -> Self {
        Self::Client(Box::new(value))
    }
}

impl StellarRpcError {
    /// Wire `errorReason` for a settle-time RPC failure.
    #[must_use]
    pub fn settle_reason(&self) -> &'static str {
        match self {
            Self::TransactionTimeout { .. } | Self::TransactionFailed { .. } => {
                "settle_exact_stellar_transaction_failed"
            }
            Self::SendRejected { .. } => "settle_exact_stellar_transaction_submission_failed",
            Self::Client(err) => match err.as_ref() {
                stellar_rpc_client::Error::TransactionSubmissionTimeout => {
                    "settle_exact_stellar_transaction_failed"
                }
                stellar_rpc_client::Error::TransactionSubmissionFailed(_) => {
                    "settle_exact_stellar_transaction_submission_failed"
                }
                _ => "unexpected_settle_error",
            },
            _ => "unexpected_settle_error",
        }
    }

    /// Transaction hash to surface on settle failure, when known.
    #[must_use]
    pub fn transaction_hash(&self) -> Option<&str> {
        match self {
            Self::TransactionTimeout { hash } | Self::TransactionFailed { hash, .. } => Some(hash),
            Self::SendRejected { hash, .. } => hash.as_deref(),
            _ => None,
        }
    }
}

/// Read/write surface used by the client and facilitator.
pub trait StellarRpc: Send + Sync {
    /// Latest closed ledger sequence.
    fn latest_ledger(&self) -> impl Future<Output = Result<u32, StellarRpcError>> + Send;

    /// Account entry for a G-address.
    fn get_account(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<AccountEntry, StellarRpcError>> + Send;

    /// Simulates a transaction envelope.
    fn simulate_transaction(
        &self,
        tx: &TransactionEnvelope,
    ) -> impl Future<Output = Result<SimulateTransactionResponse, StellarRpcError>> + Send;

    /// Submits a signed envelope and returns the transaction hash.
    fn send_transaction(
        &self,
        tx: &TransactionEnvelope,
    ) -> impl Future<Output = Result<Hash, StellarRpcError>> + Send;

    /// Fetches a previously submitted transaction.
    fn get_transaction(
        &self,
        hash: &Hash,
    ) -> impl Future<Output = Result<GetTransactionResponse, StellarRpcError>> + Send;

    /// Polls until the transaction is `SUCCESS` or `FAILED`.
    fn poll_transaction(
        &self,
        hash: &Hash,
        timeout: Duration,
    ) -> impl Future<Output = Result<GetTransactionResponse, StellarRpcError>> + Send;

    /// Estimated seconds per ledger (Horizon, falling back to 5s).
    fn estimated_ledger_seconds(&self) -> impl Future<Output = u64> + Send;
}

/// `stellar-rpc-client` 27 plus Horizon for ledger-close estimates.
#[derive(Clone, Debug)]
pub struct StellarJsonRpc {
    chain: StellarChainReference,
    rpc: Client,
    http: reqwest::Client,
    horizon_url: String,
}

impl StellarJsonRpc {
    /// Connects to `chain`, using `rpc_url` when set.
    ///
    /// Pubnet requires a non-empty `rpc_url`.
    ///
    /// # Errors
    ///
    /// Returns [`StellarRpcError`] when the URL is missing or invalid.
    pub fn connect(
        chain: StellarChainReference,
        rpc_url: Option<&str>,
    ) -> Result<Self, StellarRpcError> {
        let url = chain.rpc_url(rpc_url)?;
        let rpc = Client::new(&url)?;
        Ok(Self {
            chain,
            rpc,
            http: reqwest::Client::new(),
            horizon_url: chain.default_horizon_url().to_owned(),
        })
    }

    /// Overrides the Horizon base URL used for ledger-close estimates.
    #[must_use]
    pub fn with_horizon_url(mut self, url: impl Into<String>) -> Self {
        self.horizon_url = url.into();
        self
    }

    /// The network this client is bound to.
    #[must_use]
    pub const fn chain_reference(&self) -> StellarChainReference {
        self.chain
    }

    /// Shared official RPC handle.
    #[must_use]
    pub const fn client(&self) -> &Client {
        &self.rpc
    }
}

impl StellarRpc for StellarJsonRpc {
    fn latest_ledger(&self) -> impl Future<Output = Result<u32, StellarRpcError>> + Send {
        let rpc = self.rpc.clone();
        async move {
            let latest = rpc.get_latest_ledger().await?;
            Ok(latest.sequence)
        }
    }

    fn get_account(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<AccountEntry, StellarRpcError>> + Send {
        let rpc = self.rpc.clone();
        let address = address.to_owned();
        async move { Ok(rpc.get_account(&address).await?) }
    }

    fn simulate_transaction(
        &self,
        tx: &TransactionEnvelope,
    ) -> impl Future<Output = Result<SimulateTransactionResponse, StellarRpcError>> + Send {
        let rpc = self.rpc.clone();
        let tx = tx.clone();
        async move { Ok(rpc.simulate_transaction_envelope(&tx, None).await?) }
    }

    fn send_transaction(
        &self,
        tx: &TransactionEnvelope,
    ) -> impl Future<Output = Result<Hash, StellarRpcError>> + Send {
        let http = self.http.clone();
        let url = self.rpc.base_url().to_owned();
        let tx = tx.clone();
        async move { send_transaction_pending(&http, &url, &tx).await }
    }

    fn get_transaction(
        &self,
        hash: &Hash,
    ) -> impl Future<Output = Result<GetTransactionResponse, StellarRpcError>> + Send {
        let rpc = self.rpc.clone();
        let hash = hash.clone();
        async move { Ok(rpc.get_transaction(&hash).await?) }
    }

    fn poll_transaction(
        &self,
        hash: &Hash,
        timeout: Duration,
    ) -> impl Future<Output = Result<GetTransactionResponse, StellarRpcError>> + Send {
        let rpc = self.rpc.clone();
        let hash = hash.clone();
        async move {
            rpc.get_transaction_polling(&hash, Some(timeout))
                .await
                .map_err(|err| match err {
                    stellar_rpc_client::Error::TransactionSubmissionTimeout => {
                        StellarRpcError::TransactionTimeout {
                            hash: hash.to_string(),
                        }
                    }
                    stellar_rpc_client::Error::TransactionSubmissionFailed(detail) => {
                        StellarRpcError::TransactionFailed {
                            hash: hash.to_string(),
                            detail,
                        }
                    }
                    other => StellarRpcError::from(other),
                })
        }
    }

    fn estimated_ledger_seconds(&self) -> impl Future<Output = u64> + Send {
        let http = self.http.clone();
        let horizon_url = self.horizon_url.clone();
        async move { estimate_ledger_seconds(&http, &horizon_url).await }
    }
}

#[derive(Serialize)]
struct SendTransactionRpcRequest {
    jsonrpc: &'static str,
    id: u8,
    method: &'static str,
    params: SendTransactionRpcParams,
}

#[derive(Serialize)]
struct SendTransactionRpcParams {
    transaction: String,
}

#[derive(Deserialize)]
struct JsonRpcResponse<T> {
    result: Option<T>,
    error: Option<serde_json::Value>,
}

/// Submits via `sendTransaction` and requires `PENDING` (or `DUPLICATE`, poll-only).
async fn send_transaction_pending(
    http: &reqwest::Client,
    rpc_url: &str,
    tx: &TransactionEnvelope,
) -> Result<Hash, StellarRpcError> {
    let xdr = tx
        .to_xdr_base64(Limits::none())
        .map_err(|e| StellarRpcError::Request(e.to_string()))?;
    let body = SendTransactionRpcRequest {
        jsonrpc: "2.0",
        id: 1,
        method: "sendTransaction",
        params: SendTransactionRpcParams { transaction: xdr },
    };
    let response = http
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| StellarRpcError::Request(e.to_string()))?
        .error_for_status()
        .map_err(|e| StellarRpcError::Request(e.to_string()))?
        .json::<JsonRpcResponse<SendTransactionResponse>>()
        .await
        .map_err(|e| StellarRpcError::Request(e.to_string()))?;
    if let Some(err) = response.error {
        return Err(StellarRpcError::Request(err.to_string()));
    }
    let result = response
        .result
        .ok_or_else(|| StellarRpcError::Request("sendTransaction missing result".to_owned()))?;
    let hash = result.hash;
    match result.status.as_str() {
        "PENDING" | "DUPLICATE" => {
            Hash::from_str(&hash).map_err(|e| StellarRpcError::Request(e.to_string()))
        }
        other => Err(StellarRpcError::SendRejected {
            status: other.to_owned(),
            hash: (!hash.is_empty()).then_some(hash),
            detail: result.error_result_xdr.unwrap_or_else(|| other.to_owned()),
        }),
    }
}

#[derive(Debug, Deserialize)]
struct HorizonLedgersPage {
    #[serde(rename = "_embedded")]
    embedded: Option<HorizonEmbedded>,
}

#[derive(Debug, Deserialize)]
struct HorizonEmbedded {
    records: Vec<HorizonLedger>,
}

#[derive(Debug, Deserialize)]
struct HorizonLedger {
    closed_at: String,
}

/// Estimates seconds per ledger from the most recent Horizon ledgers.
pub async fn estimate_ledger_seconds(http: &reqwest::Client, horizon_url: &str) -> u64 {
    estimate_ledger_seconds_inner(http, horizon_url)
        .await
        .unwrap_or(DEFAULT_ESTIMATED_LEDGER_SECONDS)
}

async fn estimate_ledger_seconds_inner(
    http: &reqwest::Client,
    horizon_url: &str,
) -> Result<u64, StellarRpcError> {
    let url = format!(
        "{}/ledgers?order=desc&limit={HORIZON_LEDGERS_SAMPLE_SIZE}",
        horizon_url.trim_end_matches('/')
    );
    let page = http
        .get(url)
        .send()
        .await
        .map_err(|e| StellarRpcError::Horizon(e.to_string()))?
        .error_for_status()
        .map_err(|e| StellarRpcError::Horizon(e.to_string()))?
        .json::<HorizonLedgersPage>()
        .await
        .map_err(|e| StellarRpcError::Horizon(e.to_string()))?;
    let records = page.embedded.map(|e| e.records).unwrap_or_default();
    if records.len() < 2 {
        return Ok(DEFAULT_ESTIMATED_LEDGER_SECONDS);
    }
    let newest = records
        .first()
        .and_then(|r| parse_horizon_time(&r.closed_at))
        .ok_or_else(|| StellarRpcError::Horizon("missing newest ledger timestamp".to_owned()))?;
    let oldest = records
        .last()
        .and_then(|r| parse_horizon_time(&r.closed_at))
        .ok_or_else(|| StellarRpcError::Horizon("missing oldest ledger timestamp".to_owned()))?;
    if newest <= oldest {
        return Ok(DEFAULT_ESTIMATED_LEDGER_SECONDS);
    }
    let intervals = u64::try_from(records.len().saturating_sub(1)).unwrap_or(1);
    let seconds = (newest - oldest).div_ceil(intervals.max(1));
    Ok(seconds.max(1))
}

fn parse_horizon_time(closed_at: &str) -> Option<u64> {
    let parsed = chrono_from_rfc3339(closed_at)?;
    Some(parsed)
}

/// Minimal RFC3339 parser so we do not take `chrono` as a direct dependency.
fn chrono_from_rfc3339(value: &str) -> Option<u64> {
    // Horizon emits `2024-01-02T03:04:05Z` or with fractional seconds.
    let trimmed = value.trim();
    let (date, rest) = trimmed.split_once('T')?;
    let time = rest.trim_end_matches('Z').split('.').next().unwrap_or(rest);
    let mut date_parts = date.split('-');
    let year: i32 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    let mut time_parts = time.split(':');
    let hour: u32 = time_parts.next()?.parse().ok()?;
    let minute: u32 = time_parts.next()?.parse().ok()?;
    let second: u32 = time_parts.next()?.parse().ok()?;
    days_since_unix(year, month, day).map(|days| {
        days.saturating_mul(86_400)
            .saturating_add(u64::from(hour) * 3_600)
            .saturating_add(u64::from(minute) * 60)
            .saturating_add(u64::from(second))
    })
}

fn days_since_unix(year: i32, month: u32, day: u32) -> Option<u64> {
    const CUMULATIVE: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut days: i64 = 0;
    for y in 1970..year {
        days = days.saturating_add(if is_leap(y) { 366 } else { 365 });
    }
    let month_idx = usize::try_from(month.saturating_sub(1)).ok()?;
    let mut month_days = *CUMULATIVE.get(month_idx)?;
    if month > 2 && is_leap(year) {
        month_days = month_days.saturating_add(1);
    }
    days = days
        .saturating_add(i64::from(month_days))
        .saturating_add(i64::from(day.saturating_sub(1)));
    u64::try_from(days).ok()
}

const fn is_leap(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_horizon_timestamp() {
        assert_eq!(chrono_from_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(chrono_from_rfc3339("1970-01-01T00:00:01Z"), Some(1));
        assert!(chrono_from_rfc3339("2024-01-02T03:04:05.123Z").is_some());
    }

    #[tokio::test]
    async fn horizon_estimate_falls_back_on_empty() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/ledgers"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "_embedded": { "records": [] }
                })),
            )
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let seconds = estimate_ledger_seconds(&http, &server.uri()).await;
        assert_eq!(seconds, DEFAULT_ESTIMATED_LEDGER_SECONDS);
    }

    #[tokio::test]
    async fn horizon_estimate_from_two_ledgers() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/ledgers"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "_embedded": {
                        "records": [
                            { "closed_at": "1970-01-01T00:00:10Z" },
                            { "closed_at": "1970-01-01T00:00:00Z" }
                        ]
                    }
                })),
            )
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let seconds = estimate_ledger_seconds(&http, &server.uri()).await;
        assert_eq!(seconds, 10);
    }
}
