//! REST client abstraction for TON preflight reads and settlement.

#![allow(
    clippy::format_collect,
    clippy::missing_errors_doc,
    clippy::cast_possible_truncation,
    reason = "trace hex helpers match Toncenter JSON"
)]

use serde_json::Value;
use tonlib_core::cell::Cell;

use crate::chain::TvmAddress;
use crate::chain::codec::cell::cell_hash_base64;
use crate::chain::codec::w5::StateInitCells;

/// Account status bits needed by W5 / Highload preflight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TvmAccountState {
    /// Raw address.
    pub address: TvmAddress,
    /// Native TON balance in nanotons.
    pub balance: u128,
    /// `status == active`.
    pub is_active: bool,
    /// `status == uninit` or `nonexist`.
    pub is_uninitialized: bool,
    /// `status == frozen`.
    pub is_frozen: bool,
    /// Code + data when the account is active.
    pub state_init: Option<StateInitCells>,
}

/// TEP-74 jetton wallet getter result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TvmJettonWalletData {
    /// Jetton wallet address.
    pub address: TvmAddress,
    /// Jetton balance in atomic units.
    pub balance: u128,
    /// Wallet owner.
    pub owner: TvmAddress,
    /// Jetton master.
    pub jetton_minter: TvmAddress,
}

/// Errors from TON REST reads and submits.
#[derive(Debug, thiserror::Error)]
pub enum TvmRpcError {
    /// Transport or HTTP failure.
    #[error("tvm rpc error: {0}")]
    Rpc(String),
    /// Response JSON could not be parsed.
    #[error("tvm rpc parse error: {0}")]
    Parse(String),
}

/// REST provider selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TvmProviderKind {
    /// Toncenter REST (`/api/v3`, `/api/emulate`).
    Toncenter,
    /// TonAPI REST (`/v2`).
    Tonapi,
}

impl TvmProviderKind {
    /// Parses a provider name (`toncenter` / `tonapi`).
    ///
    /// # Errors
    ///
    /// Returns [`TvmRpcError::Parse`] if `name` is not a known provider.
    pub fn parse(name: &str) -> Result<Self, TvmRpcError> {
        match name.trim().to_ascii_lowercase().as_str() {
            "toncenter" => Ok(Self::Toncenter),
            "tonapi" => Ok(Self::Tonapi),
            other => Err(TvmRpcError::Parse(format!(
                "Unsupported TVM provider: {other}"
            ))),
        }
    }
}

/// Read/write TON RPC operations used by the client and facilitator.
pub trait TvmRpc: Send + Sync {
    /// Account state, including code/data BoCs when active.
    fn get_account_state(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<TvmAccountState, TvmRpcError>> + Send;

    /// `get_wallet_address` on the jetton minter.
    fn get_jetton_wallet(
        &self,
        asset: &str,
        owner: &str,
    ) -> impl Future<Output = Result<TvmAddress, TvmRpcError>> + Send;

    /// `get_wallet_data` on a jetton wallet.
    fn get_jetton_wallet_data(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<TvmJettonWalletData, TvmRpcError>> + Send;

    /// Emulate an external message and return a Toncenter-shaped trace.
    fn emulate_trace(
        &self,
        boc: &[u8],
        ignore_chksig: bool,
        timeout_seconds: u64,
    ) -> impl Future<Output = Result<Value, TvmRpcError>> + Send;

    /// Broadcast an external message. Returns `message_hash_norm`.
    fn send_message(&self, boc: &[u8]) -> impl Future<Output = Result<String, TvmRpcError>> + Send;

    /// Fetch a trace by external message hash.
    fn get_trace_by_message_hash(
        &self,
        message_hash: &str,
    ) -> impl Future<Output = Result<Value, TvmRpcError>> + Send;
}

impl<T: TvmRpc + Send + Sync> TvmRpc for &T {
    fn get_account_state(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<TvmAccountState, TvmRpcError>> + Send {
        (**self).get_account_state(address)
    }

    fn get_jetton_wallet(
        &self,
        asset: &str,
        owner: &str,
    ) -> impl Future<Output = Result<TvmAddress, TvmRpcError>> + Send {
        (**self).get_jetton_wallet(asset, owner)
    }

    fn get_jetton_wallet_data(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<TvmJettonWalletData, TvmRpcError>> + Send {
        (**self).get_jetton_wallet_data(address)
    }

    fn emulate_trace(
        &self,
        boc: &[u8],
        ignore_chksig: bool,
        timeout_seconds: u64,
    ) -> impl Future<Output = Result<Value, TvmRpcError>> + Send {
        (**self).emulate_trace(boc, ignore_chksig, timeout_seconds)
    }

    fn send_message(&self, boc: &[u8]) -> impl Future<Output = Result<String, TvmRpcError>> + Send {
        (**self).send_message(boc)
    }

    fn get_trace_by_message_hash(
        &self,
        message_hash: &str,
    ) -> impl Future<Output = Result<Value, TvmRpcError>> + Send {
        (**self).get_trace_by_message_hash(message_hash)
    }
}

impl<T: TvmRpc + Send + Sync> TvmRpc for std::sync::Arc<T> {
    fn get_account_state(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<TvmAccountState, TvmRpcError>> + Send {
        (**self).get_account_state(address)
    }

    fn get_jetton_wallet(
        &self,
        asset: &str,
        owner: &str,
    ) -> impl Future<Output = Result<TvmAddress, TvmRpcError>> + Send {
        (**self).get_jetton_wallet(asset, owner)
    }

    fn get_jetton_wallet_data(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<TvmJettonWalletData, TvmRpcError>> + Send {
        (**self).get_jetton_wallet_data(address)
    }

    fn emulate_trace(
        &self,
        boc: &[u8],
        ignore_chksig: bool,
        timeout_seconds: u64,
    ) -> impl Future<Output = Result<Value, TvmRpcError>> + Send {
        (**self).emulate_trace(boc, ignore_chksig, timeout_seconds)
    }

    fn send_message(&self, boc: &[u8]) -> impl Future<Output = Result<String, TvmRpcError>> + Send {
        (**self).send_message(boc)
    }

    fn get_trace_by_message_hash(
        &self,
        message_hash: &str,
    ) -> impl Future<Output = Result<Value, TvmRpcError>> + Send {
        (**self).get_trace_by_message_hash(message_hash)
    }
}

/// Errors from trace parsing / verification.
#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    /// Trace JSON is missing the transactions dict.
    #[error("{0}")]
    Invalid(String),
}

/// Collects transactions from a Toncenter-shaped trace object.
///
/// # Errors
///
/// Returns [`TraceError`] if `transactions` is missing or not an object.
pub fn parse_trace_transactions(trace: &Value) -> Result<Vec<&Value>, TraceError> {
    let transactions = trace
        .get("transactions")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            TraceError::Invalid("Toncenter trace did not return transactions dict".to_owned())
        })?;
    Ok(transactions.values().collect())
}

/// Returns `true` when compute/action phases succeeded and the tx was not aborted.
#[must_use]
pub fn transaction_succeeded(transaction: &Value) -> bool {
    let description = transaction
        .get("description")
        .or(Some(transaction))
        .and_then(Value::as_object);
    let Some(description) = description else {
        return false;
    };
    if description.get("aborted").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    let compute = description.get("compute_ph").and_then(Value::as_object);
    let Some(compute) = compute else {
        return false;
    };
    if compute.get("skipped").and_then(Value::as_bool) == Some(true)
        || compute.get("success").and_then(Value::as_bool) != Some(true)
    {
        return false;
    }
    if let Some(action) = description.get("action").and_then(Value::as_object)
        && action.get("success").and_then(Value::as_bool) != Some(true)
    {
        return false;
    }
    true
}

/// Returns `true` if `message.message_content.hash` equals the cell hash (standard base64).
#[must_use]
pub fn message_body_hash_matches(message: Option<&Value>, expected: &Cell) -> bool {
    let Some(message) = message else {
        return false;
    };
    let hash = message
        .get("message_content")
        .and_then(|c| c.get("hash"))
        .and_then(Value::as_str);
    hash == Some(cell_hash_base64(expected).as_str())
}

/// Decodes a trace tx hash (hex or base64) to lowercase hex.
///
/// # Errors
///
/// Returns [`TraceError`] if the string is neither 64-hex nor base64.
pub fn trace_transaction_hash_to_hex(encoded: &str) -> Result<String, TraceError> {
    let normalized = encoded.trim();
    if normalized.len() == 64 && normalized.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(normalized.to_ascii_lowercase());
    }
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, normalized)
        .map_err(|e| TraceError::Invalid(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Sum of `out_msgs[].fwd_fee`, falling back to action-phase totals.
#[must_use]
pub fn trace_transaction_fwd_fees(transaction: &Value, expected_count: u128) -> u128 {
    let out_msgs = transaction
        .get("out_msgs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let exact: Vec<u128> = out_msgs
        .iter()
        .filter_map(|m| parse_integer(m.get("fwd_fee")))
        .collect();
    if !exact.is_empty() {
        return exact.iter().sum();
    }
    let action = transaction
        .get("description")
        .and_then(|d| d.get("action"))
        .or_else(|| transaction.get("action"));
    if let Some(action) = action {
        if let Some(total) = parse_integer(action.get("total_fwd_fees")) {
            return total;
        }
        if let Some(fwd) = parse_integer(action.get("fwd_fee")) {
            return fwd.saturating_mul(expected_count);
        }
    }
    0
}

/// Compute-phase `gas_fees`.
#[must_use]
pub fn trace_transaction_compute_fees(transaction: &Value) -> u128 {
    parse_integer(
        transaction
            .get("description")
            .and_then(|d| d.get("compute_ph"))
            .and_then(|c| c.get("gas_fees"))
            .or_else(|| transaction.get("gas_fees")),
    )
    .unwrap_or(0)
}

/// Storage fees collected + due.
#[must_use]
pub fn trace_transaction_storage_fees(transaction: &Value) -> u128 {
    let storage = transaction
        .get("description")
        .and_then(|d| d.get("storage_ph"))
        .or_else(|| transaction.get("storage_ph"));
    let collected =
        parse_integer(storage.and_then(|s| s.get("storage_fees_collected"))).unwrap_or(0);
    let due = parse_integer(storage.and_then(|s| s.get("storage_fees_due"))).unwrap_or(0);
    collected.saturating_add(due)
}

/// Tries to parse a TON address from a JSON string.
#[must_use]
pub fn normalize_address_or_null(value: Option<&Value>) -> Option<TvmAddress> {
    value
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse().ok())
}

fn parse_integer(value: Option<&Value>) -> Option<u128> {
    match value? {
        Value::Number(n) => n.as_u64().map(u128::from),
        Value::Bool(true) => Some(1),
        Value::Bool(false) => Some(0),
        Value::String(s) if !s.is_empty() => s.parse().ok(),
        _ => None,
    }
}
