//! TonAPI REST mapping onto the Toncenter-shaped trace/stack model.

#![allow(
    clippy::excessive_nesting,
    clippy::cognitive_complexity,
    clippy::too_many_lines,
    clippy::missing_const_for_fn,
    clippy::or_fun_call,
    clippy::option_if_let_else,
    clippy::redundant_closure,
    clippy::needless_question_mark,
    clippy::manual_let_else,
    clippy::type_complexity,
    clippy::collapsible_if,
    clippy::format_collect,
    clippy::missing_errors_doc,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::map_unwrap_or,
    reason = "TON REST sequential JSON mapping"
)]

use serde_json::{Value, json};
use tonlib_core::TonAddress;
use tonlib_core::cell::BagOfCells;
use tonlib_core::message::{CommonMsgInfo, TonMessage, TransferMessage};

use super::toncenter::parse_stack_cell;
use crate::chain::codec::cell::{decode_boc_text, encode_base64_boc, normalize_address};
use crate::chain::defaults::{EXTERNAL_SIGNED_OP, INTERNAL_SIGNED_OP, JETTON_TRANSFER_OP};
use crate::chain::{TvmAddress, TvmRpcError};

pub(crate) fn tonapi_get_method_arg(item: &Value) -> Result<Value, TvmRpcError> {
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

pub(crate) fn tonapi_stack_record_to_toncenter(record: &Value) -> Result<Value, TvmRpcError> {
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
                let b64 =
                    encode_base64_boc(&cell).map_err(|e| TvmRpcError::Parse(e.to_string()))?;
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

pub(crate) fn tonapi_trace_to_toncenter(trace: &Value) -> Value {
    let mut transactions = serde_json::Map::new();
    walk_tonapi_trace(trace, &mut transactions);
    json!({ "transactions": transactions, "is_incomplete": false })
}

pub(crate) fn walk_tonapi_trace(
    node: &Value,
    out: &mut serde_json::Map<String, Value>,
) -> Option<Value> {
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

pub(crate) fn tonapi_transaction_to_toncenter(transaction: &Value) -> Value {
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

pub(crate) fn tonapi_message_to_toncenter(message: Option<&Value>) -> Value {
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

pub(crate) fn tonapi_account_address(value: Option<&Value>) -> String {
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

pub(crate) fn normalize_decoded_opcode(message: &Value) -> String {
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

pub(crate) fn normalized_external_message_hash_hex(boc: &[u8]) -> Result<String, TvmRpcError> {
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
