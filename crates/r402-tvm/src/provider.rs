//! Toncenter / TonAPI REST clients.

use std::time::Duration;

use base64::Engine;
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{Value, json};
use tonlib_core::TonAddress;
use tonlib_core::cell::BagOfCells;
use tonlib_core::message::{CommonMsgInfo, TonMessage, TransferMessage};
use url::Url;

use crate::chain::{
    TvmAccountState, TvmAddress, TvmChainReference, TvmJettonWalletData, TvmProviderKind, TvmRpc,
    TvmRpcError,
};
use crate::codecs::common::{address_to_stack_item, decode_boc_text, normalize_address};
use crate::codecs::jetton::coins_to_u128;
use crate::codecs::w5::StateInitCells;
use crate::{
    DEFAULT_TONCENTER_TIMEOUT_SECONDS, EXTERNAL_SIGNED_OP, INTERNAL_SIGNED_OP, JETTON_TRANSFER_OP,
    TONAPI_MAINNET_BASE_URL, TONAPI_TESTNET_BASE_URL, TONCENTER_MAINNET_BASE_URL,
    TONCENTER_TESTNET_BASE_URL,
};

/// HTTP REST client for Toncenter or TonAPI.
#[derive(Clone, Debug)]
pub struct TvmRestClient {
    kind: TvmProviderKind,
    root: Url,
    headers: HeaderMap,
    timeout: Duration,
    http: reqwest::Client,
}

impl TvmRestClient {
    /// Connects to the default endpoint for `chain` / `kind`.
    ///
    /// # Errors
    ///
    /// Returns [`TvmRpcError`] if `base_url` is not a valid URL.
    pub fn connect(
        chain: TvmChainReference,
        kind: TvmProviderKind,
        api_key: Option<&str>,
        base_url: Option<&str>,
        timeout_seconds: Option<u64>,
    ) -> Result<Self, TvmRpcError> {
        let root = base_url.unwrap_or(default_base_url(chain, kind));
        let mut url = Url::parse(root).map_err(|e| TvmRpcError::Parse(e.to_string()))?;
        let trimmed = url.path().trim_end_matches('/').to_owned();
        url.set_path(&trimmed);
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static("application/json"),
        );
        if let Some(key) = api_key.filter(|k| !k.is_empty()) {
            match kind {
                TvmProviderKind::Toncenter => {
                    headers.insert(
                        "X-Api-Key",
                        HeaderValue::from_str(key)
                            .map_err(|e| TvmRpcError::Parse(e.to_string()))?,
                    );
                }
                TvmProviderKind::Tonapi => {
                    let value = format!("Bearer {key}");
                    headers.insert(
                        reqwest::header::AUTHORIZATION,
                        HeaderValue::from_str(&value)
                            .map_err(|e| TvmRpcError::Parse(e.to_string()))?,
                    );
                }
            }
        }
        let timeout =
            Duration::from_secs(timeout_seconds.unwrap_or(DEFAULT_TONCENTER_TIMEOUT_SECONDS));
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| TvmRpcError::Rpc(e.to_string()))?;
        Ok(Self {
            kind,
            root: url,
            headers,
            timeout,
            http,
        })
    }
}

/// Default REST root for `chain` / `provider`.
#[must_use]
pub fn default_base_url(chain: TvmChainReference, kind: TvmProviderKind) -> &'static str {
    match (kind, chain) {
        (TvmProviderKind::Toncenter, TvmChainReference::Mainnet) => TONCENTER_MAINNET_BASE_URL,
        (TvmProviderKind::Toncenter, TvmChainReference::Testnet) => TONCENTER_TESTNET_BASE_URL,
        (TvmProviderKind::Tonapi, TvmChainReference::Mainnet) => TONAPI_MAINNET_BASE_URL,
        (TvmProviderKind::Tonapi, TvmChainReference::Testnet) => TONAPI_TESTNET_BASE_URL,
    }
}

impl TvmRpc for TvmRestClient {
    async fn get_account_state(&self, address: &str) -> Result<TvmAccountState, TvmRpcError> {
        let normalized =
            normalize_address(address).map_err(|e| TvmRpcError::Parse(e.to_string()))?;
        match self.kind {
            TvmProviderKind::Toncenter => {
                let body = self
                    .request(
                        reqwest::Method::GET,
                        "/api/v3/accountStates",
                        &[("address", normalized.as_str()), ("include_boc", "true")],
                        None,
                        self.timeout,
                    )
                    .await?;
                let accounts = body
                    .get("accounts")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let Some(account) = accounts.first() else {
                    return Ok(synthetic_uninit(normalized));
                };
                account_state_from_payload(&normalized, account, "code_boc", "data_boc")
            }
            TvmProviderKind::Tonapi => {
                let path = format!("/v2/blockchain/accounts/{}", normalized.as_str());
                match self
                    .request(reqwest::Method::GET, &path, &[], None, self.timeout)
                    .await
                {
                    Ok(account) => {
                        account_state_from_payload(&normalized, &account, "code", "data")
                    }
                    Err(TvmRpcError::Rpc(msg)) if msg.contains(" 404") => {
                        Ok(synthetic_uninit(normalized))
                    }
                    Err(err) => Err(err),
                }
            }
        }
    }

    async fn get_jetton_wallet(&self, asset: &str, owner: &str) -> Result<TvmAddress, TvmRpcError> {
        let owner_ton = TonAddress::from_str_checked(owner)?;
        let stack =
            vec![address_to_stack_item(&owner_ton).map_err(|e| TvmRpcError::Parse(e.to_string()))?];
        let result = self
            .run_get_method(asset, "get_wallet_address", stack)
            .await?;
        let item = result
            .first()
            .ok_or_else(|| TvmRpcError::Parse("empty get_wallet_address stack".to_owned()))?;
        parse_stack_address(item)
    }

    async fn get_jetton_wallet_data(
        &self,
        address: &str,
    ) -> Result<TvmJettonWalletData, TvmRpcError> {
        let result = self
            .run_get_method(address, "get_wallet_data", Vec::new())
            .await?;
        if result.len() < 3 {
            return Err(TvmRpcError::Parse(
                "get_wallet_data returned an incomplete stack".to_owned(),
            ));
        }
        Ok(TvmJettonWalletData {
            address: normalize_address(address).map_err(|e| TvmRpcError::Parse(e.to_string()))?,
            balance: parse_stack_num(result.first())?,
            owner: parse_stack_address(result.get(1).unwrap_or(&Value::Null))?,
            jetton_minter: parse_stack_address(result.get(2).unwrap_or(&Value::Null))?,
        })
    }

    async fn emulate_trace(
        &self,
        boc: &[u8],
        ignore_chksig: bool,
        timeout_seconds: u64,
    ) -> Result<Value, TvmRpcError> {
        let b64 = base64::engine::general_purpose::STANDARD.encode(boc);
        let timeout = Duration::from_secs(timeout_seconds.max(1));
        match self.kind {
            TvmProviderKind::Toncenter => {
                self.request(
                    reqwest::Method::POST,
                    "/api/emulate/v1/emulateTrace",
                    &[],
                    Some(json!({
                        "boc": b64,
                        "ignore_chksig": ignore_chksig,
                        "with_actions": true,
                    })),
                    timeout,
                )
                .await
            }
            TvmProviderKind::Tonapi => {
                let raw = self
                    .request(
                        reqwest::Method::POST,
                        "/v2/traces/emulate",
                        &[(
                            "ignore_signature_check",
                            if ignore_chksig { "true" } else { "false" },
                        )],
                        Some(json!({ "boc": b64 })),
                        timeout,
                    )
                    .await?;
                Ok(tonapi_trace_to_toncenter(&raw))
            }
        }
    }

    async fn send_message(&self, boc: &[u8]) -> Result<String, TvmRpcError> {
        let b64 = base64::engine::general_purpose::STANDARD.encode(boc);
        match self.kind {
            TvmProviderKind::Toncenter => {
                let response = self
                    .request(
                        reqwest::Method::POST,
                        "/api/v3/message",
                        &[],
                        Some(json!({ "boc": b64 })),
                        self.timeout,
                    )
                    .await?;
                Ok(response
                    .get("message_hash_norm")
                    .or_else(|| response.get("message_hash"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned())
            }
            TvmProviderKind::Tonapi => {
                self.request(
                    reqwest::Method::POST,
                    "/v2/blockchain/message",
                    &[],
                    Some(json!({ "boc": b64 })),
                    self.timeout,
                )
                .await?;
                normalized_external_message_hash_hex(boc)
            }
        }
    }

    async fn get_trace_by_message_hash(&self, message_hash: &str) -> Result<Value, TvmRpcError> {
        match self.kind {
            TvmProviderKind::Toncenter => {
                let response = self
                    .request(
                        reqwest::Method::GET,
                        "/api/v3/traces",
                        &[("msg_hash", message_hash), ("limit", "1"), ("sort", "desc")],
                        None,
                        self.timeout,
                    )
                    .await?;
                let traces = response
                    .get("traces")
                    .and_then(Value::as_array)
                    .ok_or_else(|| TvmRpcError::Parse("invalid traces response".to_owned()))?;
                traces
                    .iter()
                    .find(|t| t.is_object())
                    .cloned()
                    .ok_or_else(|| {
                        TvmRpcError::Parse(format!(
                            "Toncenter returned no trace for message hash {message_hash}"
                        ))
                    })
            }
            TvmProviderKind::Tonapi => {
                let path = format!("/v2/traces/{message_hash}");
                let raw = self
                    .request(reqwest::Method::GET, &path, &[], None, self.timeout)
                    .await?;
                Ok(tonapi_trace_to_toncenter(&raw))
            }
        }
    }
}

impl TvmRestClient {
    async fn run_get_method(
        &self,
        address: &str,
        method: &str,
        stack: Vec<Value>,
    ) -> Result<Vec<Value>, TvmRpcError> {
        let normalized =
            normalize_address(address).map_err(|e| TvmRpcError::Parse(e.to_string()))?;
        match self.kind {
            TvmProviderKind::Toncenter => {
                let response = self
                    .request(
                        reqwest::Method::POST,
                        "/api/v3/runGetMethod",
                        &[],
                        Some(json!({
                            "address": normalized.as_str(),
                            "method": method,
                            "stack": stack,
                        })),
                        self.timeout,
                    )
                    .await?;
                let exit = response
                    .get("exit_code")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                if exit != 0 {
                    return Err(TvmRpcError::Rpc(format!(
                        "Toncenter get-method {method} failed with exit code {exit}"
                    )));
                }
                Ok(response
                    .get("stack")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|item| item.is_object())
                    .collect())
            }
            TvmProviderKind::Tonapi => {
                let path = format!(
                    "/v2/blockchain/accounts/{}/methods/{method}",
                    normalized.as_str()
                );
                let args: Vec<Value> = stack
                    .iter()
                    .map(tonapi_get_method_arg)
                    .collect::<Result<_, _>>()?;
                let response = self
                    .request(
                        reqwest::Method::POST,
                        &path,
                        &[],
                        Some(json!({ "args": args })),
                        self.timeout,
                    )
                    .await?;
                let exit = response
                    .get("exit_code")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                if response.get("success").and_then(Value::as_bool) == Some(false) || exit != 0 {
                    return Err(TvmRpcError::Rpc(format!(
                        "TonAPI get-method {method} failed with exit code {exit}"
                    )));
                }
                Ok(response
                    .get("stack")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|item| item.is_object())
                    .map(|item| tonapi_stack_record_to_toncenter(&item))
                    .collect::<Result<_, _>>()?)
            }
        }
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, &str)],
        json_body: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, TvmRpcError> {
        let mut last_err = TvmRpcError::Rpc("request failed".to_owned());
        for attempt in 0..5 {
            let mut url = self
                .root
                .join(path.trim_start_matches('/'))
                .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
            {
                let mut pairs = url.query_pairs_mut();
                for (k, v) in query {
                    pairs.append_pair(k, v);
                }
            }
            let mut builder = self
                .http
                .request(method.clone(), url)
                .headers(self.headers.clone())
                .timeout(timeout);
            if let Some(body) = &json_body {
                builder = builder.json(body);
            }
            match builder.send().await {
                Ok(response) => {
                    let status = response.status();
                    if !status.is_success() {
                        let retryable = matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504);
                        last_err = TvmRpcError::Rpc(format!("{path}: {status}"));
                        if retryable && attempt < 4 {
                            tokio::time::sleep(Duration::from_millis(250 * (attempt + 1))).await;
                            continue;
                        }
                        return Err(last_err);
                    }
                    let text = response
                        .text()
                        .await
                        .map_err(|e| TvmRpcError::Rpc(e.to_string()))?;
                    if text.is_empty() {
                        return Ok(json!({}));
                    }
                    let data: Value = serde_json::from_str(&text)
                        .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
                    if !data.is_object() {
                        return Err(TvmRpcError::Parse(format!(
                            "non-object response for {path}"
                        )));
                    }
                    return Ok(data);
                }
                Err(err) => {
                    last_err = TvmRpcError::Rpc(err.to_string());
                    if attempt < 4 {
                        tokio::time::sleep(Duration::from_millis(250 * (attempt + 1))).await;
                        continue;
                    }
                }
            }
        }
        Err(last_err)
    }
}

fn synthetic_uninit(address: TvmAddress) -> TvmAccountState {
    TvmAccountState {
        address,
        balance: 0,
        is_active: false,
        is_uninitialized: true,
        is_frozen: false,
        state_init: None,
    }
}

fn account_state_from_payload(
    address: &TvmAddress,
    account: &Value,
    code_key: &str,
    data_key: &str,
) -> Result<TvmAccountState, TvmRpcError> {
    let status = account.get("status").and_then(Value::as_str).unwrap_or("");
    let mut state_init = None;
    if status == "active" {
        if let (Some(code), Some(data)) = (
            account.get(code_key).and_then(Value::as_str),
            account.get(data_key).and_then(Value::as_str),
        ) {
            let code_cell = boc_from_text(code)?;
            let data_cell = boc_from_text(data)?;
            state_init = Some(StateInitCells {
                code: code_cell,
                data: data_cell,
            });
        }
    }
    let balance = account
        .get("balance")
        .map(json_u128)
        .transpose()?
        .unwrap_or(0);
    Ok(TvmAccountState {
        address: address.clone(),
        balance,
        is_active: status == "active",
        is_uninitialized: status == "uninit" || status == "nonexist",
        is_frozen: status == "frozen",
        state_init,
    })
}

fn boc_from_text(value: &str) -> Result<tonlib_core::cell::Cell, TvmRpcError> {
    let bytes = decode_boc_text(value).map_err(|e| TvmRpcError::Parse(e.to_string()))?;
    let boc = BagOfCells::parse(&bytes).map_err(|e| TvmRpcError::Parse(e.to_string()))?;
    let root = boc
        .single_root()
        .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
    Ok((*root).clone())
}

fn parse_stack_address(item: &Value) -> Result<TvmAddress, TvmRpcError> {
    let cell = parse_stack_cell(item)?;
    let mut parser = cell.parser();
    let addr = parser
        .load_address()
        .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
    TvmAddress::try_from(&addr).map_err(|e| TvmRpcError::Parse(e.to_string()))
}

fn parse_stack_cell(item: &Value) -> Result<tonlib_core::cell::Cell, TvmRpcError> {
    let value = item
        .get("value")
        .and_then(Value::as_str)
        .ok_or_else(|| TvmRpcError::Parse("Can't parse cell stack value".to_owned()))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
    let boc = BagOfCells::parse(&bytes).map_err(|e| TvmRpcError::Parse(e.to_string()))?;
    let root = boc
        .single_root()
        .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
    Ok((*root).clone())
}

fn parse_stack_num(item: Option<&Value>) -> Result<u128, TvmRpcError> {
    let item = item.ok_or_else(|| TvmRpcError::Parse("missing stack num".to_owned()))?;
    json_u128(item.get("value").unwrap_or(item))
}

fn json_u128(value: &Value) -> Result<u128, TvmRpcError> {
    match value {
        Value::Number(n) => n
            .as_u64()
            .map(u128::from)
            .ok_or_else(|| TvmRpcError::Parse("invalid number".to_owned())),
        Value::String(s) => {
            let t = s.trim();
            let t = t.strip_prefix("0x").unwrap_or(t);
            if t.bytes().all(|b| b.is_ascii_hexdigit())
                && t.starts_with(|c: char| c.is_ascii_hexdigit())
                && s.trim().starts_with("0x")
            {
                u128::from_str_radix(t, 16).map_err(|e| TvmRpcError::Parse(e.to_string()))
            } else if t.bytes().all(|b| b.is_ascii_digit()) {
                t.parse::<u128>()
                    .map_err(|e| TvmRpcError::Parse(e.to_string()))
            } else {
                coins_to_u128(
                    &t.parse::<num_bigint::BigUint>()
                        .map_err(|e| TvmRpcError::Parse(e.to_string()))?,
                )
                .map_err(|e| TvmRpcError::Parse(e.to_string()))
            }
        }
        _ => Err(TvmRpcError::Parse("invalid integer".to_owned())),
    }
}

fn tonapi_get_method_arg(item: &Value) -> Result<Value, TvmRpcError> {
    let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
    let value = item
        .get("value")
        .ok_or_else(|| TvmRpcError::Parse("stack item missing value".to_owned()))?;
    match item_type {
        "slice" => {
            if let Some(s) = value.as_str() {
                if let Ok(cell) = parse_stack_cell(item)
                    && let Ok(addr) = cell.parser().load_address()
                {
                    let raw = TvmAddress::try_from(&addr)
                        .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
                    return Ok(json!({ "type": "slice", "value": raw.as_str() }));
                }
                let bytes = decode_boc_text(s).map_err(|e| TvmRpcError::Parse(e.to_string()))?;
                return Ok(json!({
                    "type": "slice_boc_hex",
                    "value": bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
                }));
            }
        }
        "cell" => {
            return Ok(json!({ "type": "cell_boc_base64", "value": value }));
        }
        "num" | "int" => {
            return Ok(json!({ "type": "int257", "value": value }));
        }
        other => {
            return Ok(json!({ "type": other, "value": value }));
        }
    }
    Err(TvmRpcError::Parse(format!(
        "Unsupported TonAPI get-method stack item type: {item_type}"
    )))
}

fn tonapi_stack_record_to_toncenter(record: &Value) -> Result<Value, TvmRpcError> {
    let record_type = record.get("type").and_then(Value::as_str).unwrap_or("");
    match record_type {
        "num" => Ok(json!({
            "type": "num",
            "value": record.get("num").cloned().unwrap_or(json!("0")),
        })),
        "null" => Ok(json!({ "type": "null", "value": Value::Null })),
        "cell" => Ok(json!({
            "type": "cell",
            "value": record.get("cell").cloned().unwrap_or(Value::Null),
        })),
        "slice" => {
            if let Some(s) = record.get("slice").and_then(Value::as_str)
                && let Ok(addr) = s.parse::<TonAddress>()
            {
                let cell = tonlib_core::cell::CellBuilder::new()
                    .store_address(&addr)
                    .and_then(tonlib_core::cell::CellBuilder::build)
                    .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
                let b64 = crate::codecs::common::encode_base64_boc(&cell)
                    .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
                return Ok(json!({ "type": "slice", "value": b64 }));
            }
            Ok(json!({
                "type": "slice",
                "value": record.get("slice").cloned().unwrap_or(Value::Null),
            }))
        }
        _ => Err(TvmRpcError::Parse(format!(
            "TonAPI returned an unsupported stack record: {record}"
        ))),
    }
}

fn tonapi_trace_to_toncenter(trace: &Value) -> Value {
    let mut transactions = serde_json::Map::new();
    walk_tonapi_trace(trace, &mut transactions);
    json!({ "transactions": transactions, "is_incomplete": false })
}

fn walk_tonapi_trace(node: &Value, out: &mut serde_json::Map<String, Value>) -> Option<Value> {
    let converted = node.get("transaction").map(tonapi_transaction_to_toncenter);
    if let Some(ref converted) = converted {
        let hash = converted
            .get("hash")
            .and_then(Value::as_str)
            .map_or_else(|| out.len().to_string(), ToOwned::to_owned);
        out.insert(hash, converted.clone());
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            if let Some(child_tx) = walk_tonapi_trace(child, out)
                && let Some(parent) = converted.as_ref()
            {
                let _ = (parent, child_tx);
            }
        }
    }
    converted
}

fn tonapi_transaction_to_toncenter(transaction: &Value) -> Value {
    json!({
        "account": tonapi_account_address(transaction.get("account")),
        "hash": transaction.get("hash").cloned().unwrap_or(json!("")),
        "hash_norm": transaction.get("hash").cloned().unwrap_or(json!("")),
        "description": {
            "aborted": transaction.get("aborted"),
            "compute_ph": transaction.get("compute_phase"),
            "action": transaction.get("action_phase"),
            "storage_ph": transaction.get("storage_phase"),
        },
        "in_msg": tonapi_message_to_toncenter(transaction.get("in_msg")),
        "out_msgs": transaction.get("out_msgs").and_then(Value::as_array).map(|msgs| {
            msgs.iter().map(|m| tonapi_message_to_toncenter(Some(m))).collect::<Vec<_>>()
        }),
    })
}

fn tonapi_message_to_toncenter(message: Option<&Value>) -> Value {
    let Some(record) = message else {
        return json!({});
    };
    let opcode = normalize_decoded_opcode(record);
    json!({
        "hash": record.get("hash").cloned().unwrap_or(json!("")),
        "hash_norm": record.get("hash").cloned().unwrap_or(json!("")),
        "source": tonapi_account_address(record.get("source")),
        "destination": tonapi_account_address(record.get("destination")),
        "decoded_opcode": opcode,
        "fwd_fee": record.get("fwd_fee"),
        "value": record.get("value"),
        "message_content": {
            "decoded": record.get("decoded_body"),
        }
    })
}

fn tonapi_account_address(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) if !s.is_empty() => normalize_address(s)
            .map(|a| a.to_string())
            .unwrap_or_default(),
        Some(Value::Object(map)) => map
            .get("address")
            .and_then(Value::as_str)
            .and_then(|s| normalize_address(s).ok())
            .map(|a| a.to_string())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn normalize_decoded_opcode(message: &Value) -> String {
    if let Some(name) = message.get("decoded_op_name").and_then(Value::as_str)
        && !name.is_empty()
    {
        return name
            .chars()
            .enumerate()
            .flat_map(|(i, c)| {
                if i > 0 && c.is_uppercase() {
                    vec!['_', c.to_ascii_lowercase()]
                } else {
                    vec![c.to_ascii_lowercase()]
                }
            })
            .collect();
    }
    let opcode = message.get("op_code").and_then(|v| match v {
        Value::Number(n) => n.as_u64().map(|n| n as u32),
        Value::String(s) => {
            let t = s.trim();
            if let Some(hex) = t.strip_prefix("0x") {
                u32::from_str_radix(hex, 16).ok()
            } else {
                t.parse().ok()
            }
        }
        _ => None,
    });
    match opcode {
        Some(JETTON_TRANSFER_OP) => "jetton_transfer".to_owned(),
        Some(0x178d_4519) => "jetton_internal_transfer".to_owned(),
        Some(INTERNAL_SIGNED_OP) => "w5_internal_signed_request".to_owned(),
        Some(EXTERNAL_SIGNED_OP) => "w5_external_signed_request".to_owned(),
        Some(op) => format!("0x{op:x}"),
        None => String::new(),
    }
}

fn normalized_external_message_hash_hex(boc: &[u8]) -> Result<String, TvmRpcError> {
    let cells = BagOfCells::parse(boc).map_err(|e| TvmRpcError::Parse(e.to_string()))?;
    let root = cells
        .single_root()
        .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
    let message = TransferMessage::parse(&root).map_err(|e| TvmRpcError::Parse(e.to_string()))?;
    if !matches!(
        message.common_msg_info,
        CommonMsgInfo::ExternalIncomingMessage(_)
    ) {
        return Ok(root.cell_hash().to_hex());
    }
    let dest = message.common_msg_info.dest();
    let mut builder = tonlib_core::cell::CellBuilder::new();
    builder
        .store_u8(2, 2)
        .and_then(|b| b.store_address(&TonAddress::NULL))
        .and_then(|b| b.store_address(&dest))
        .and_then(|b| b.store_coins(&num_bigint::BigUint::from(0u32)))
        .and_then(|b| b.store_bit(false))
        .and_then(|b| b.store_bit(true))
        .and_then(|b| b.store_reference(&message.body))
        .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
    let rebuilt = builder
        .build()
        .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
    Ok(rebuilt.cell_hash().to_hex())
}

trait TonAddressParse {
    fn from_str_checked(s: &str) -> Result<TonAddress, TvmRpcError>;
}

impl TonAddressParse for TonAddress {
    fn from_str_checked(s: &str) -> Result<TonAddress, TvmRpcError> {
        s.parse()
            .map_err(|e: tonlib_core::TonAddressParseError| TvmRpcError::Parse(e.to_string()))
    }
}
