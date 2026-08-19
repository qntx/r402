//! JSON-RPC HTTP client for XRPL reads, simulation, and submit.

use std::fmt::{Debug, Formatter};
use std::time::Duration;

use serde_json::{Value, json};
use url::Url;

use crate::chain::types::XrplChainReference;

/// Default poll interval while waiting for a validated `tx` result.
const SUBMIT_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Errors from XRPL JSON-RPC reads and submits.
#[derive(Debug, thiserror::Error)]
pub enum XrplRpcError {
    /// HTTP or JSON-RPC transport failure.
    #[error("xrpl rpc error: {0}")]
    Rpc(String),
    /// Response JSON could not be parsed.
    #[error("xrpl rpc parse error: {0}")]
    Parse(String),
}

/// Signing authorization state for an XRPL account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XrplAccountAuthorization {
    /// Classic address of the configured regular key, if any.
    pub regular_key: Option<String>,
    /// Whether `lsfDisableMaster` is set on the account.
    pub is_master_key_disabled: bool,
    /// Current account `Sequence`.
    pub sequence: u32,
}

/// Result of submitting a signed blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XrplSubmitResult {
    /// Transaction hash.
    pub hash: String,
    /// Engine result from `submit` (preliminary).
    pub engine_result: String,
}

/// Validated `tx` lookup result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XrplTxResult {
    /// Transaction hash.
    pub hash: String,
    /// Whether the result is from a validated ledger.
    pub validated: bool,
    /// `meta.TransactionResult` when present.
    pub result_code: String,
    /// Transaction metadata JSON, if present.
    pub meta: Option<Value>,
}

/// Result of `simulate` on an unsigned transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XrplSimulationResult {
    /// XRPL engine result code.
    pub engine_result: String,
    /// Human-readable engine result.
    pub engine_result_message: Option<String>,
}

/// Read and submit operations used by the client and facilitator.
pub trait XrplRpc: Send + Sync {
    /// Current validated ledger index.
    fn current_ledger_index(&self) -> impl Future<Output = Result<u32, XrplRpcError>> + Send;

    /// Validated `account_info` for `account`.
    fn account_authorization(
        &self,
        account: &str,
    ) -> impl Future<Output = Result<XrplAccountAuthorization, XrplRpcError>> + Send;

    /// Available ticket sequences for `account`, ascending.
    fn ticket_sequences(
        &self,
        account: &str,
    ) -> impl Future<Output = Result<Vec<u32>, XrplRpcError>> + Send;

    /// Open-ledger fee in drops.
    fn fee_drops(&self) -> impl Future<Output = Result<u64, XrplRpcError>> + Send;

    /// Simulates an unsigned transaction JSON object.
    fn simulate(
        &self,
        unsigned_tx: &Value,
    ) -> impl Future<Output = Result<XrplSimulationResult, XrplRpcError>> + Send;

    /// Submits a signed blob with `fail_hard`.
    fn submit(
        &self,
        signed_tx_blob: &str,
    ) -> impl Future<Output = Result<XrplSubmitResult, XrplRpcError>> + Send;

    /// Looks up a transaction by hash.
    fn tx(
        &self,
        hash: &str,
    ) -> impl Future<Output = Result<Option<XrplTxResult>, XrplRpcError>> + Send;
}

/// JSON-RPC HTTP client pointed at an XRPL node.
#[derive(Clone)]
pub struct XrplJsonRpc {
    url: String,
    http: reqwest::Client,
}

impl Debug for XrplJsonRpc {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XrplJsonRpc")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl XrplJsonRpc {
    /// Connects to the given JSON-RPC URL.
    #[must_use]
    pub fn connect(url: impl AsRef<str>) -> Self {
        Self {
            url: url.as_ref().to_owned(),
            http: reqwest::Client::new(),
        }
    }

    /// Connects using the default URL for `chain`, or `rpc_url` when set.
    ///
    /// # Errors
    ///
    /// Returns [`XrplRpcError::Parse`] when `chain` has no default URL and
    /// `rpc_url` is `None`.
    pub fn connect_chain(
        chain: XrplChainReference,
        rpc_url: Option<String>,
    ) -> Result<Self, XrplRpcError> {
        let url = match rpc_url {
            Some(url) => url,
            None => chain
                .default_rpc_url()
                .map(ToOwned::to_owned)
                .ok_or_else(|| {
                    XrplRpcError::Parse(format!(
                        "no default xrpl rpc url for network {}",
                        chain.network_id()
                    ))
                })?,
        };
        let _ = Url::parse(&url).map_err(|e| XrplRpcError::Parse(e.to_string()))?;
        Ok(Self::connect(url))
    }

    /// RPC endpoint URL.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, XrplRpcError> {
        let body = json!({
            "method": method,
            "params": [params],
        });
        let response = self
            .http
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .map_err(|e| XrplRpcError::Rpc(e.to_string()))?;
        let value: Value = response
            .json()
            .await
            .map_err(|e| XrplRpcError::Parse(e.to_string()))?;
        if let Some(message) = json_rpc_error_message(&value) {
            return Err(XrplRpcError::Rpc(message));
        }
        value
            .get("result")
            .cloned()
            .ok_or_else(|| XrplRpcError::Parse("missing rpc result".to_owned()))
    }

    /// Submits `signed_tx_blob` and polls `tx` until validated or `last_ledger` passes.
    ///
    /// # Errors
    ///
    /// Returns [`XrplRpcError`] on transport failure or when the transaction
    /// never validates.
    pub async fn submit_and_wait(
        &self,
        signed_tx_blob: &str,
        hash: &str,
        last_ledger: u32,
    ) -> Result<XrplTxResult, XrplRpcError> {
        poll_until_validated(self, signed_tx_blob, hash, last_ledger).await
    }
}

/// Submits a signed blob and polls `tx` until validated or `LastLedgerSequence` passes.
///
/// # Errors
///
/// Returns [`XrplRpcError`] on transport failure or when the transaction expires.
pub async fn submit_and_wait<R: XrplRpc>(
    rpc: &R,
    signed_tx_blob: &str,
    hash: &str,
    last_ledger: u32,
) -> Result<XrplTxResult, XrplRpcError> {
    poll_until_validated(rpc, signed_tx_blob, hash, last_ledger).await
}

async fn poll_until_validated<R: XrplRpc>(
    rpc: &R,
    signed_tx_blob: &str,
    hash: &str,
    last_ledger: u32,
) -> Result<XrplTxResult, XrplRpcError> {
    let submitted = rpc.submit(signed_tx_blob).await?;
    if is_hard_fail(&submitted.engine_result) {
        return Ok(XrplTxResult {
            hash: submitted.hash,
            validated: false,
            result_code: submitted.engine_result,
            meta: None,
        });
    }
    loop {
        if let Some(tx) = rpc.tx(hash).await?
            && tx.validated
        {
            return Ok(tx);
        }
        let current = rpc.current_ledger_index().await?;
        if current > last_ledger {
            return Err(XrplRpcError::Rpc(format!(
                "transaction {hash} expired after LastLedgerSequence {last_ledger}"
            )));
        }
        tokio::time::sleep(SUBMIT_POLL_INTERVAL).await;
    }
}

fn json_rpc_error_message(envelope: &Value) -> Option<String> {
    if let Some(err) = envelope.get("error")
        && !err.is_null()
    {
        return Some(
            envelope
                .get("error_message")
                .and_then(Value::as_str)
                .or_else(|| err.as_str())
                .unwrap_or("xrpl rpc error")
                .to_owned(),
        );
    }
    let result = envelope.get("result")?;
    let nested = result.get("error");
    let nested_status = result.get("status").and_then(Value::as_str);
    if nested_status == Some("error") || nested.is_some_and(|e| !e.is_null()) {
        return Some(
            nested
                .and_then(Value::as_str)
                .or_else(|| result.get("error_message").and_then(Value::as_str))
                .unwrap_or("xrpl rpc error")
                .to_owned(),
        );
    }
    None
}

fn is_txn_missing(message: &str) -> bool {
    message.contains("txnNotFound") || message.contains("actNotFound")
}

fn ticket_sequence_from_object(object: &Value) -> Option<u32> {
    if object.get("LedgerEntryType").and_then(Value::as_str) != Some("Ticket") {
        return None;
    }
    object
        .get("TicketSequence")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
}

fn is_hard_fail(engine_result: &str) -> bool {
    engine_result.starts_with("tem")
        || engine_result.starts_with("tef")
        || engine_result.starts_with("tel")
}

fn parse_tx_result(hash: &str, result: &Value) -> XrplTxResult {
    let validated = result
        .get("validated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let result_code = result
        .get("meta")
        .and_then(|meta| {
            if let Some(code) = meta.get("TransactionResult").and_then(Value::as_str) {
                return Some(code.to_owned());
            }
            None
        })
        .unwrap_or_else(|| "unknown".to_owned());
    XrplTxResult {
        hash: result
            .get("hash")
            .and_then(Value::as_str)
            .unwrap_or(hash)
            .to_owned(),
        validated,
        result_code,
        meta: result.get("meta").cloned(),
    }
}

impl XrplRpc for XrplJsonRpc {
    async fn current_ledger_index(&self) -> Result<u32, XrplRpcError> {
        let result = self
            .call("ledger", json!({"ledger_index": "validated"}))
            .await?;
        result
            .get("ledger_index")
            .and_then(Value::as_u64)
            .or_else(|| {
                result
                    .get("ledger")
                    .and_then(|ledger| ledger.get("ledger_index"))
                    .and_then(|idx| {
                        idx.as_u64()
                            .or_else(|| idx.as_str().and_then(|s| s.parse().ok()))
                    })
            })
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| XrplRpcError::Parse("missing ledger_index".to_owned()))
    }

    async fn account_authorization(
        &self,
        account: &str,
    ) -> Result<XrplAccountAuthorization, XrplRpcError> {
        let result = self
            .call(
                "account_info",
                json!({
                    "account": account,
                    "ledger_index": "validated",
                }),
            )
            .await?;
        let data = result
            .get("account_data")
            .ok_or_else(|| XrplRpcError::Parse("missing account_data".to_owned()))?;
        let sequence = data
            .get("Sequence")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| XrplRpcError::Parse("missing Sequence".to_owned()))?;
        let flags = data.get("Flags").and_then(Value::as_u64).unwrap_or(0);
        let regular_key = data
            .get("RegularKey")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        Ok(XrplAccountAuthorization {
            regular_key,
            is_master_key_disabled: (flags & u64::from(crate::LSF_DISABLE_MASTER)) != 0,
            sequence,
        })
    }

    async fn ticket_sequences(&self, account: &str) -> Result<Vec<u32>, XrplRpcError> {
        let mut sequences = Vec::new();
        let mut marker: Option<Value> = None;
        loop {
            let mut params = json!({
                "account": account,
                "type": "ticket",
                "ledger_index": "validated",
            });
            if let Some(marker_value) = marker.take()
                && let Some(obj) = params.as_object_mut()
            {
                obj.insert("marker".to_owned(), marker_value);
            }
            let result = self.call("account_objects", params).await?;
            let objects = result
                .get("account_objects")
                .and_then(Value::as_array)
                .into_iter()
                .flatten();
            sequences.extend(objects.filter_map(ticket_sequence_from_object));
            match result.get("marker").cloned() {
                Some(next) if !next.is_null() => marker = Some(next),
                _ => break,
            }
        }
        sequences.sort_unstable();
        Ok(sequences)
    }

    async fn fee_drops(&self) -> Result<u64, XrplRpcError> {
        let result = self.call("fee", json!({})).await?;
        let fee = result
            .get("drops")
            .and_then(|drops| drops.get("open_ledger_fee"))
            .ok_or_else(|| XrplRpcError::Parse("missing open_ledger_fee".to_owned()))?;
        match fee {
            Value::String(s) => s
                .parse()
                .map_err(|_| XrplRpcError::Parse(format!("invalid open_ledger_fee: {s}"))),
            Value::Number(n) => n
                .as_u64()
                .ok_or_else(|| XrplRpcError::Parse("invalid open_ledger_fee".to_owned())),
            _ => Err(XrplRpcError::Parse("invalid open_ledger_fee".to_owned())),
        }
    }

    async fn simulate(&self, unsigned_tx: &Value) -> Result<XrplSimulationResult, XrplRpcError> {
        let result = self
            .call("simulate", json!({ "tx_json": unsigned_tx }))
            .await?;
        let engine_result = result
            .get("engine_result")
            .and_then(Value::as_str)
            .ok_or_else(|| XrplRpcError::Parse("missing engine_result".to_owned()))?
            .to_owned();
        let engine_result_message = result
            .get("engine_result_message")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        Ok(XrplSimulationResult {
            engine_result,
            engine_result_message,
        })
    }

    async fn submit(&self, signed_tx_blob: &str) -> Result<XrplSubmitResult, XrplRpcError> {
        let result = self
            .call(
                "submit",
                json!({
                    "tx_blob": signed_tx_blob,
                    "fail_hard": true,
                }),
            )
            .await?;
        let engine_result = result
            .get("engine_result")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let hash = result
            .get("tx_json")
            .and_then(|tx| tx.get("hash"))
            .and_then(Value::as_str)
            .or_else(|| result.get("hash").and_then(Value::as_str))
            .map(ToOwned::to_owned)
            .ok_or_else(|| XrplRpcError::Parse("missing submit hash".to_owned()))?;
        Ok(XrplSubmitResult {
            hash,
            engine_result,
        })
    }

    async fn tx(&self, hash: &str) -> Result<Option<XrplTxResult>, XrplRpcError> {
        match self.call("tx", json!({ "transaction": hash })).await {
            Ok(result) => Ok(Some(parse_tx_result(hash, &result))),
            Err(XrplRpcError::Rpc(message)) if is_txn_missing(&message) => Ok(None),
            Err(err) => Err(err),
        }
    }
}

impl<T: XrplRpc + Send + Sync> XrplRpc for &T {
    fn current_ledger_index(&self) -> impl Future<Output = Result<u32, XrplRpcError>> + Send {
        (**self).current_ledger_index()
    }

    fn account_authorization(
        &self,
        account: &str,
    ) -> impl Future<Output = Result<XrplAccountAuthorization, XrplRpcError>> + Send {
        (**self).account_authorization(account)
    }

    fn ticket_sequences(
        &self,
        account: &str,
    ) -> impl Future<Output = Result<Vec<u32>, XrplRpcError>> + Send {
        (**self).ticket_sequences(account)
    }

    fn fee_drops(&self) -> impl Future<Output = Result<u64, XrplRpcError>> + Send {
        (**self).fee_drops()
    }

    fn simulate(
        &self,
        unsigned_tx: &Value,
    ) -> impl Future<Output = Result<XrplSimulationResult, XrplRpcError>> + Send {
        (**self).simulate(unsigned_tx)
    }

    fn submit(
        &self,
        signed_tx_blob: &str,
    ) -> impl Future<Output = Result<XrplSubmitResult, XrplRpcError>> + Send {
        (**self).submit(signed_tx_blob)
    }

    fn tx(
        &self,
        hash: &str,
    ) -> impl Future<Output = Result<Option<XrplTxResult>, XrplRpcError>> + Send {
        (**self).tx(hash)
    }
}

impl<T: XrplRpc + Send + Sync> XrplRpc for std::sync::Arc<T> {
    fn current_ledger_index(&self) -> impl Future<Output = Result<u32, XrplRpcError>> + Send {
        (**self).current_ledger_index()
    }

    fn account_authorization(
        &self,
        account: &str,
    ) -> impl Future<Output = Result<XrplAccountAuthorization, XrplRpcError>> + Send {
        (**self).account_authorization(account)
    }

    fn ticket_sequences(
        &self,
        account: &str,
    ) -> impl Future<Output = Result<Vec<u32>, XrplRpcError>> + Send {
        (**self).ticket_sequences(account)
    }

    fn fee_drops(&self) -> impl Future<Output = Result<u64, XrplRpcError>> + Send {
        (**self).fee_drops()
    }

    fn simulate(
        &self,
        unsigned_tx: &Value,
    ) -> impl Future<Output = Result<XrplSimulationResult, XrplRpcError>> + Send {
        (**self).simulate(unsigned_tx)
    }

    fn submit(
        &self,
        signed_tx_blob: &str,
    ) -> impl Future<Output = Result<XrplSubmitResult, XrplRpcError>> + Send {
        (**self).submit(signed_tx_blob)
    }

    fn tx(
        &self,
        hash: &str,
    ) -> impl Future<Output = Result<Option<XrplTxResult>, XrplRpcError>> + Send {
        (**self).tx(hash)
    }
}

/// Strips signature fields so the remaining JSON can be passed to `simulate`.
#[must_use]
pub fn unsigned_tx_for_simulate(decoded: &Value) -> Value {
    let mut unsigned = decoded.clone();
    if let Some(obj) = unsigned.as_object_mut() {
        let _ = obj.remove("TxnSignature");
        let _ = obj.remove("SigningPubKey");
        let _ = obj.remove("Signers");
    }
    unsigned
}

/// Ticket sequences created by a validated `TicketCreate`.
#[must_use]
pub fn extract_created_ticket_sequences(meta: &Value) -> Vec<u32> {
    let mut sequences = Vec::new();
    let Some(nodes) = meta.get("AffectedNodes").and_then(Value::as_array) else {
        return sequences;
    };
    for node in nodes {
        let Some(created) = node.get("CreatedNode") else {
            continue;
        };
        if created.get("LedgerEntryType").and_then(Value::as_str) != Some("Ticket") {
            continue;
        }
        if let Some(seq) = created
            .get("NewFields")
            .and_then(|fields| fields.get("TicketSequence"))
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
        {
            sequences.push(seq);
        }
    }
    sequences.sort_unstable();
    sequences
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]
mod tests {
    use serde_json::json;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    use super::*;

    struct LedgerRespond;

    impl Respond for LedgerRespond {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or_else(|_| json!({}));
            let rpc_method = body.get("method").and_then(Value::as_str).unwrap_or("");
            let result = match rpc_method {
                "ledger" => json!({ "ledger_index": 990, "validated": true }),
                "account_info" => json!({
                    "account_data": {
                        "Account": "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh",
                        "Sequence": 7,
                        "Flags": 0
                    },
                    "validated": true
                }),
                other => json!({ "error": format!("unexpected {other}") }),
            };
            ResponseTemplate::new(200).set_body_json(json!({
                "result": result,
                "status": "success",
                "type": "response"
            }))
        }
    }

    #[tokio::test]
    async fn ledger_and_account_info_via_json_rpc() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(LedgerRespond)
            .mount(&server)
            .await;
        let rpc = XrplJsonRpc::connect(server.uri());
        assert_eq!(rpc.current_ledger_index().await.unwrap(), 990);
        let auth = rpc
            .account_authorization("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh")
            .await
            .unwrap();
        assert_eq!(auth.sequence, 7);
        assert!(!auth.is_master_key_disabled);
    }

    struct TxNotFoundRespond;

    impl Respond for TxNotFoundRespond {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or_else(|_| json!({}));
            let rpc_method = body.get("method").and_then(Value::as_str).unwrap_or("");
            match rpc_method {
                "tx" => ResponseTemplate::new(200).set_body_json(json!({
                    "result": {
                        "error": "txnNotFound",
                        "status": "error",
                        "error_message": "Transaction not found."
                    }
                })),
                _ => ResponseTemplate::new(200).set_body_json(json!({
                    "result": { "error": format!("unexpected {rpc_method}") },
                    "status": "success"
                })),
            }
        }
    }

    #[tokio::test]
    async fn tx_maps_nested_txn_not_found_to_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(TxNotFoundRespond)
            .mount(&server)
            .await;
        let rpc = XrplJsonRpc::connect(server.uri());
        assert!(rpc.tx("AB".repeat(32).as_str()).await.unwrap().is_none());
    }

    #[test]
    fn extract_tickets_from_meta() {
        let meta = json!({
            "AffectedNodes": [
                {"CreatedNode": {"LedgerEntryType": "Ticket", "NewFields": {"TicketSequence": 8}}},
                {"CreatedNode": {"LedgerEntryType": "Ticket", "NewFields": {"TicketSequence": 4}}}
            ]
        });
        assert_eq!(extract_created_ticket_sequences(&meta), vec![4, 8]);
    }
}
