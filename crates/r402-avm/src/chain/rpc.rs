//! Algod REST client: suggested params, simulate, broadcast, pending.

use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter};
use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::Deserialize;

use super::codec::{CodecError, MpValue, encode_value};

/// Suggested transaction parameters from `GET /v2/transactions/params`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestedParams {
    /// Fee per byte in µAlgo (`fee`). Zero under normal conditions.
    pub fee_per_byte: u64,
    /// Protocol minimum fee in µAlgo (`min-fee`).
    pub min_fee: u64,
    /// 32-byte genesis hash.
    pub genesis_hash: [u8; 32],
    /// Genesis ID (`mainnet-v1.0` / `testnet-v1.0`).
    pub genesis_id: String,
    /// Latest committed round.
    pub last_round: u64,
}

/// Outcome of `POST /v2/transactions/simulate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulateResult {
    /// First group failure message, if the group would not commit.
    pub failure_message: Option<String>,
}

/// Outcome of `GET /v2/transactions/pending/{txid}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTransaction {
    /// Confirmed round when the txn has landed.
    pub confirmed_round: Option<u64>,
    /// Pool rejection reason, empty when still pending or confirmed.
    pub pool_error: String,
}

/// Consecutive `pending_transaction` failures before `wait_for_confirmation` aborts.
#[cfg(feature = "facilitator")]
const PENDING_ERROR_BUDGET: u32 = 3;

/// Errors from algod REST reads and submits.
#[derive(Debug, Clone, thiserror::Error)]
pub enum AlgodError {
    /// HTTP transport or non-success status.
    #[error("algod error: {0}")]
    Http(String),
    /// Response JSON could not be parsed.
    #[error("algod parse error: {0}")]
    Parse(String),
    /// Pending txn is not yet in the pool (`404`).
    #[error("algod pending transaction not found")]
    NotFound,
}

impl From<CodecError> for AlgodError {
    fn from(value: CodecError) -> Self {
        Self::Parse(value.to_string())
    }
}

/// Read/write algod operations used by the client and facilitator.
pub trait AlgodRpc: Send + Sync {
    /// `GET /v2/transactions/params`.
    fn suggested_params(&self) -> impl Future<Output = Result<SuggestedParams, AlgodError>> + Send;

    /// `POST /v2/transactions/simulate` for one atomic group.
    fn simulate_group(
        &self,
        signed_txns: &[Vec<u8>],
    ) -> impl Future<Output = Result<SimulateResult, AlgodError>> + Send;

    /// `POST /v2/transactions` with concatenated signed txn bytes.
    fn send_group(
        &self,
        signed_txns: &[Vec<u8>],
    ) -> impl Future<Output = Result<String, AlgodError>> + Send;

    /// `GET /v2/transactions/pending/{txid}`.
    fn pending_transaction(
        &self,
        txid: &str,
    ) -> impl Future<Output = Result<PendingTransaction, AlgodError>> + Send;

    /// Latest committed round (`GET /v2/status`).
    fn last_round(&self) -> impl Future<Output = Result<u64, AlgodError>> + Send;
}

/// HTTP client pointed at an algod endpoint.
#[derive(Clone)]
pub struct AlgodClient {
    http: reqwest::Client,
    base_url: String,
    token: Option<String>,
}

impl Debug for AlgodClient {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlgodClient")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

fn is_loopback_url(url: &str) -> bool {
    url.contains("://127.0.0.1")
        || url.contains("://localhost")
        || url.contains("://[::1]")
        || url.contains("://::1")
}

impl AlgodClient {
    /// Connects to `base_url`. `token` is sent as `X-Algo-API-Token` when set.
    #[must_use]
    pub fn connect(base_url: impl Into<String>, token: Option<String>) -> Self {
        let base_url = base_url.into();
        let mut builder = reqwest::Client::builder();
        if is_loopback_url(&base_url) {
            // HTTP_PROXY must not capture loopback mock servers or local algod.
            builder = builder.no_proxy();
        }
        let http = builder.build().unwrap_or_else(|_| reqwest::Client::new());
        Self {
            http,
            base_url,
            token,
        }
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    fn headers(&self, content_type: &str) -> Result<HeaderMap, AlgodError> {
        let mut headers = HeaderMap::new();
        let ct =
            HeaderValue::from_str(content_type).map_err(|e| AlgodError::Parse(e.to_string()))?;
        let _ = headers.insert(CONTENT_TYPE, ct);
        if let Some(token) = &self.token
            && !token.is_empty()
        {
            let value =
                HeaderValue::from_str(token).map_err(|e| AlgodError::Parse(e.to_string()))?;
            let _ = headers.insert("X-Algo-API-Token", value);
        }
        Ok(headers)
    }
}

#[derive(Debug, Deserialize)]
struct ParamsResponse {
    fee: u64,
    #[serde(rename = "min-fee")]
    min_fee: u64,
    #[serde(rename = "genesis-hash")]
    genesis_hash: String,
    #[serde(rename = "genesis-id")]
    genesis_id: String,
    #[serde(rename = "last-round")]
    last_round: u64,
}

#[derive(Debug, Deserialize)]
struct SendResponse {
    #[serde(rename = "txId", alias = "txid")]
    tx_id: String,
}

#[derive(Debug, Deserialize)]
struct StatusResponse {
    #[serde(rename = "last-round")]
    last_round: u64,
}

impl AlgodRpc for AlgodClient {
    async fn suggested_params(&self) -> Result<SuggestedParams, AlgodError> {
        let response = self
            .http
            .get(self.url("/v2/transactions/params"))
            .headers(self.headers("application/json")?)
            .send()
            .await
            .map_err(|e| AlgodError::Http(e.to_string()))?;
        if !response.status().is_success() {
            return Err(AlgodError::Http(format!(
                "params status {}",
                response.status()
            )));
        }
        let body: ParamsResponse = response
            .json()
            .await
            .map_err(|e| AlgodError::Parse(e.to_string()))?;
        let hash_bytes = {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.decode(body.genesis_hash.trim())
        }
        .map_err(|e| AlgodError::Parse(e.to_string()))?;
        let genesis_hash: [u8; 32] = hash_bytes
            .try_into()
            .map_err(|_| AlgodError::Parse("genesis-hash is not 32 bytes".to_owned()))?;
        Ok(SuggestedParams {
            fee_per_byte: body.fee,
            min_fee: body.min_fee,
            genesis_hash,
            genesis_id: body.genesis_id,
            last_round: body.last_round,
        })
    }

    async fn simulate_group(&self, signed_txns: &[Vec<u8>]) -> Result<SimulateResult, AlgodError> {
        let mut txns = Vec::with_capacity(signed_txns.len());
        for raw in signed_txns {
            txns.push(super::codec::decode_value(raw)?);
        }
        let mut group = BTreeMap::new();
        let _ = group.insert("txns".to_owned(), MpValue::Array(txns));
        let mut root = BTreeMap::new();
        let _ = root.insert(
            "txn-groups".to_owned(),
            MpValue::Array(vec![MpValue::Map(group)]),
        );
        let body = encode_value(&MpValue::Map(root));
        let response = self
            .http
            .post(self.url("/v2/transactions/simulate"))
            .headers(self.headers("application/msgpack")?)
            .body(body)
            .send()
            .await
            .map_err(|e| AlgodError::Http(e.to_string()))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AlgodError::Http(format!(
                "simulate status {status}: {text}"
            )));
        }
        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AlgodError::Parse(e.to_string()))?;
        Ok(SimulateResult {
            failure_message: first_failure_message(&json),
        })
    }

    async fn send_group(&self, signed_txns: &[Vec<u8>]) -> Result<String, AlgodError> {
        let mut body = Vec::new();
        for raw in signed_txns {
            body.extend_from_slice(raw);
        }
        let response = self
            .http
            .post(self.url("/v2/transactions"))
            .headers(self.headers("application/x-binary")?)
            .body(body)
            .send()
            .await
            .map_err(|e| AlgodError::Http(e.to_string()))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AlgodError::Http(format!(
                "broadcast status {status}: {text}"
            )));
        }
        let parsed: SendResponse = response
            .json()
            .await
            .map_err(|e| AlgodError::Parse(e.to_string()))?;
        Ok(parsed.tx_id)
    }

    async fn pending_transaction(&self, txid: &str) -> Result<PendingTransaction, AlgodError> {
        let path = format!("/v2/transactions/pending/{txid}");
        let response = self
            .http
            .get(self.url(&path))
            .headers(self.headers("application/json")?)
            .send()
            .await
            .map_err(|e| AlgodError::Http(e.to_string()))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(AlgodError::NotFound);
        }
        if !response.status().is_success() {
            return Err(AlgodError::Http(format!(
                "pending status {}",
                response.status()
            )));
        }
        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AlgodError::Parse(e.to_string()))?;
        let confirmed_round = json
            .get("confirmed-round")
            .or_else(|| json.get("confirmedRound"))
            .and_then(serde_json::Value::as_u64)
            .filter(|r| *r > 0);
        let pool_error = json
            .get("pool-error")
            .or_else(|| json.get("poolError"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned();
        Ok(PendingTransaction {
            confirmed_round,
            pool_error,
        })
    }

    async fn last_round(&self) -> Result<u64, AlgodError> {
        let response = self
            .http
            .get(self.url("/v2/status"))
            .headers(self.headers("application/json")?)
            .send()
            .await
            .map_err(|e| AlgodError::Http(e.to_string()))?;
        if !response.status().is_success() {
            return Err(AlgodError::Http(format!(
                "status status {}",
                response.status()
            )));
        }
        let body: StatusResponse = response
            .json()
            .await
            .map_err(|e| AlgodError::Parse(e.to_string()))?;
        Ok(body.last_round)
    }
}

/// Polls pending until confirmation or `last-round` advances by `wait_rounds`.
///
/// `404` (not yet in the pool) is treated as still pending. Other pending
/// RPC errors abort after `PENDING_ERROR_BUDGET` consecutive failures.
///
/// # Errors
///
/// Returns [`AlgodError`] when the pool rejects the txn, pending RPC errors
/// persist, or `wait_rounds` rounds elapse without confirmation.
#[cfg(feature = "facilitator")]
pub async fn wait_for_confirmation<R: AlgodRpc>(
    rpc: &R,
    txid: &str,
    wait_rounds: u32,
) -> Result<(), AlgodError> {
    let start = rpc.last_round().await?;
    let limit = start.saturating_add(u64::from(wait_rounds.max(1)));
    let mut consecutive_errors = 0_u32;
    loop {
        match rpc.pending_transaction(txid).await {
            Ok(pending) if !pending.pool_error.is_empty() => {
                return Err(AlgodError::Http(format!(
                    "pool error: {}",
                    pending.pool_error
                )));
            }
            Ok(pending) if pending.confirmed_round.is_some() => return Ok(()),
            Ok(_) | Err(AlgodError::NotFound) => {
                consecutive_errors = 0;
            }
            Err(err) => {
                consecutive_errors = consecutive_errors.saturating_add(1);
                if consecutive_errors >= PENDING_ERROR_BUDGET {
                    return Err(err);
                }
            }
        }
        let current = rpc.last_round().await?;
        if current >= limit {
            return Err(AlgodError::Http(format!("confirmation timeout for {txid}")));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn first_failure_message(json: &serde_json::Value) -> Option<String> {
    let groups = json
        .get("txn-groups")
        .or_else(|| json.get("txnGroups"))
        .and_then(serde_json::Value::as_array)?;
    let first = groups.first()?;
    let message = first
        .get("failure-message")
        .or_else(|| first.get("failureMessage"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if message.is_empty() {
        None
    } else {
        Some(message.to_owned())
    }
}

impl<T: AlgodRpc + Send + Sync> AlgodRpc for &T {
    fn suggested_params(&self) -> impl Future<Output = Result<SuggestedParams, AlgodError>> + Send {
        (**self).suggested_params()
    }

    fn simulate_group(
        &self,
        signed_txns: &[Vec<u8>],
    ) -> impl Future<Output = Result<SimulateResult, AlgodError>> + Send {
        (**self).simulate_group(signed_txns)
    }

    fn send_group(
        &self,
        signed_txns: &[Vec<u8>],
    ) -> impl Future<Output = Result<String, AlgodError>> + Send {
        (**self).send_group(signed_txns)
    }

    fn pending_transaction(
        &self,
        txid: &str,
    ) -> impl Future<Output = Result<PendingTransaction, AlgodError>> + Send {
        (**self).pending_transaction(txid)
    }

    fn last_round(&self) -> impl Future<Output = Result<u64, AlgodError>> + Send {
        (**self).last_round()
    }
}

impl<T: AlgodRpc + Send + Sync> AlgodRpc for std::sync::Arc<T> {
    fn suggested_params(&self) -> impl Future<Output = Result<SuggestedParams, AlgodError>> + Send {
        (**self).suggested_params()
    }

    fn simulate_group(
        &self,
        signed_txns: &[Vec<u8>],
    ) -> impl Future<Output = Result<SimulateResult, AlgodError>> + Send {
        (**self).simulate_group(signed_txns)
    }

    fn send_group(
        &self,
        signed_txns: &[Vec<u8>],
    ) -> impl Future<Output = Result<String, AlgodError>> + Send {
        (**self).send_group(signed_txns)
    }

    fn pending_transaction(
        &self,
        txid: &str,
    ) -> impl Future<Output = Result<PendingTransaction, AlgodError>> + Send {
        (**self).pending_transaction(txid)
    }

    fn last_round(&self) -> impl Future<Output = Result<u64, AlgodError>> + Send {
        (**self).last_round()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[test]
    fn failure_message_kebab_and_camel() {
        let kebab = serde_json::json!({
            "txn-groups": [{ "failure-message": "overspend" }]
        });
        assert_eq!(first_failure_message(&kebab).as_deref(), Some("overspend"));
        let camel = serde_json::json!({
            "txnGroups": [{ "failureMessage": "" }]
        });
        assert!(first_failure_message(&camel).is_none());
    }

    #[tokio::test]
    async fn suggested_params_via_http() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/transactions/params"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "fee": 0,
                "min-fee": 1000,
                "genesis-hash": "SGO1GKSzyE7IEPItTxCByw9x8FmnrCDexi9/cOUJOiI=",
                "genesis-id": "testnet-v1.0",
                "last-round": 42,
                "consensus-version": "test"
            })))
            .mount(&server)
            .await;
        let rpc = AlgodClient::connect(server.uri(), None);
        let params = rpc.suggested_params().await.unwrap();
        assert_eq!(params.last_round, 42);
        assert_eq!(params.min_fee, 1000);
        assert_eq!(
            params.genesis_hash,
            crate::chain::account::ALGORAND_TESTNET_GENESIS_HASH
        );
    }

    #[cfg(feature = "facilitator")]
    mod wait {
        use std::sync::atomic::{AtomicU64, Ordering};

        use super::*;

        struct WaitMock {
            last: AtomicU64,
            pending: Result<PendingTransaction, AlgodError>,
            pending_calls: AtomicU64,
        }

        impl AlgodRpc for WaitMock {
            async fn suggested_params(&self) -> Result<SuggestedParams, AlgodError> {
                Err(AlgodError::Http("unused".to_owned()))
            }

            async fn simulate_group(
                &self,
                _signed_txns: &[Vec<u8>],
            ) -> Result<SimulateResult, AlgodError> {
                Err(AlgodError::Http("unused".to_owned()))
            }

            async fn send_group(&self, _signed_txns: &[Vec<u8>]) -> Result<String, AlgodError> {
                Err(AlgodError::Http("unused".to_owned()))
            }

            async fn pending_transaction(
                &self,
                _txid: &str,
            ) -> Result<PendingTransaction, AlgodError> {
                let _ = self.pending_calls.fetch_add(1, Ordering::SeqCst);
                self.pending.clone()
            }

            async fn last_round(&self) -> Result<u64, AlgodError> {
                Ok(self.last.fetch_add(1, Ordering::SeqCst))
            }
        }

        #[tokio::test]
        async fn times_out_after_last_round_advances() {
            let rpc = WaitMock {
                last: AtomicU64::new(10),
                pending: Ok(PendingTransaction {
                    confirmed_round: None,
                    pool_error: String::new(),
                }),
                pending_calls: AtomicU64::new(0),
            };
            let err = wait_for_confirmation(&rpc, "TX", 2).await.unwrap_err();
            assert!(err.to_string().contains("confirmation timeout"), "{err}");
        }

        #[tokio::test]
        async fn surfaces_persistent_pending_errors() {
            let rpc = WaitMock {
                last: AtomicU64::new(10),
                pending: Err(AlgodError::Http("boom".to_owned())),
                pending_calls: AtomicU64::new(0),
            };
            let err = wait_for_confirmation(&rpc, "TX", 20).await.unwrap_err();
            assert!(err.to_string().contains("boom"), "{err}");
            assert!(rpc.pending_calls.load(Ordering::SeqCst) >= u64::from(PENDING_ERROR_BUDGET));
        }

        #[tokio::test]
        async fn treats_not_found_as_pending() {
            let rpc = WaitMock {
                last: AtomicU64::new(10),
                pending: Err(AlgodError::NotFound),
                pending_calls: AtomicU64::new(0),
            };
            let err = wait_for_confirmation(&rpc, "TX", 1).await.unwrap_err();
            assert!(err.to_string().contains("confirmation timeout"), "{err}");
            assert!(rpc.pending_calls.load(Ordering::SeqCst) >= 1);
        }
    }
}
