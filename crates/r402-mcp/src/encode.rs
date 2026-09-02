//! Transport encoding for MCP × x402.
//!
//! Ports Go `go/mcp/utils.go` and `specs/transports-v2/mcp.md`:
//! - payment-required: dual `structuredContent` + `content[0].text`
//! - settlement failure uses the **same** dual format (Go server R5)
//! - meta keys `x402/payment` / `x402/payment-response`

use r402_core::wire::{
    Extensions, PaymentPayload, PaymentRequired, PaymentRequirements, SettleResponse,
};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock, RequestMetaObject};
use serde_json::Value;

use crate::error::McpCallError;
use crate::{
    JSONRPC_PAYMENT_REQUIRED_CODE, MCP_PAYMENT_META_KEY, MCP_PAYMENT_REQUIRED_CODE,
    MCP_PAYMENT_RESPONSE_META_KEY, MCP_TOOL_URL_PREFIX,
};

/// Wire payment payload shape used on MCP `_meta` (opaque scheme payload).
pub type McpPaymentPayload = PaymentPayload<PaymentRequirements, Value>;

/// Builds a tool resource URL: custom if non-empty, else `mcp://tool/{name}`.
///
/// Go: `CreateToolResourceUrl`.
#[must_use]
pub fn create_tool_resource_url(tool_name: &str, custom_url: Option<&str>) -> String {
    match custom_url {
        Some(u) if !u.is_empty() => u.to_owned(),
        _ => format!("{MCP_TOOL_URL_PREFIX}{tool_name}"),
    }
}

/// Payment-required tool result per transport spec / Go `paymentRequiredResult`.
///
/// Sets `isError: true`, `structuredContent` = `PaymentRequired`, and
/// `content[0].text` = JSON string of the same object.
#[must_use]
pub fn payment_required_tool_result(required: &PaymentRequired) -> CallToolResult {
    let value = serde_json::to_value(required).unwrap_or(Value::Null);
    CallToolResult::structured_error(value)
}

/// Settlement-failed result (Go R5: same dual format as payment-required).
///
/// Go `settlementFailedResult` delegates to `paymentRequiredResult`.
#[must_use]
pub fn settlement_failed_tool_result(
    accepts: &[PaymentRequirements],
    resource: &r402_core::wire::ResourceInfo,
    extensions: &Extensions,
    error_msg: impl Into<String>,
) -> CallToolResult {
    let mut required = PaymentRequired::new(resource.clone())
        .with_error(error_msg.into())
        .with_accepts(accepts.to_vec());
    if !extensions.is_empty() {
        required = required.with_extensions(extensions.clone());
    }
    payment_required_tool_result(&required)
}

/// Prefer `structuredContent`, else parse first text content as JSON.
///
/// Go `paymentRequiredObject` / `ExtractPaymentRequiredFromResult`:
/// requires `x402Version` present and non-empty `accepts`; V2 only for us.
#[must_use]
pub fn extract_payment_required(result: &CallToolResult) -> Option<PaymentRequired> {
    if !result.is_error.unwrap_or(false) {
        return None;
    }
    if let Some(ref sc) = result.structured_content
        && let Some(pr) = payment_required_from_value(sc)
    {
        return Some(pr);
    }
    result
        .content
        .iter()
        .find_map(ContentBlock::as_text)
        .and_then(|t| serde_json::from_str::<Value>(&t.text).ok())
        .and_then(|v| payment_required_from_value(&v))
}

fn payment_required_from_value(value: &Value) -> Option<PaymentRequired> {
    let obj = value.as_object()?;
    // Go isPaymentRequiredObject: accepts + x402Version
    if !obj.contains_key("x402Version") || !obj.contains_key("accepts") {
        return None;
    }
    let version = obj.get("x402Version").and_then(Value::as_u64)?;
    if version != 2 {
        return None;
    }
    let accepts = obj.get("accepts")?.as_array()?;
    if accepts.is_empty() {
        return None;
    }
    serde_json::from_value(value.clone()).ok()
}

/// Reads `_meta["x402/payment"]` from tool-call params.
///
/// Go `ExtractPaymentFromMeta`: invalid structure → None (not error).
#[must_use]
pub fn extract_payment_from_params(params: &CallToolRequestParams) -> Option<McpPaymentPayload> {
    let meta = params.meta.as_ref()?;
    let value = meta.get(MCP_PAYMENT_META_KEY)?.clone();
    let payload: McpPaymentPayload = serde_json::from_value(value).ok()?;
    // Go utils: require version and payload present
    if payload.payload.is_null() {
        return None;
    }
    Some(payload)
}

/// Writes `_meta["x402/payment"]` onto tool-call params.
#[must_use]
pub fn attach_payment_to_params(
    mut params: CallToolRequestParams,
    payload: &McpPaymentPayload,
) -> CallToolRequestParams {
    let value = serde_json::to_value(payload).unwrap_or(Value::Null);
    let meta = params.meta.get_or_insert_with(RequestMetaObject::new);
    meta.insert(MCP_PAYMENT_META_KEY.to_owned(), value);
    params
}

/// Reads settlement response from result `_meta`.
#[must_use]
pub fn extract_settle_response(result: &CallToolResult) -> Option<SettleResponse> {
    let meta = result.meta.as_ref()?;
    let value = meta.get(MCP_PAYMENT_RESPONSE_META_KEY)?.clone();
    serde_json::from_value(value).ok()
}

/// Attaches settlement response under `_meta["x402/payment-response"]`.
#[must_use]
pub fn attach_settle_response(
    mut result: CallToolResult,
    settle: &SettleResponse,
) -> CallToolResult {
    let value = serde_json::to_value(settle).unwrap_or(Value::Null);
    let mut meta = result.meta.take().unwrap_or_default();
    meta.insert(MCP_PAYMENT_RESPONSE_META_KEY.to_owned(), value);
    result.meta = Some(meta);
    result
}

/// True when the tool result is an x402 payment-required challenge.
#[must_use]
pub fn is_payment_required_result(result: &CallToolResult) -> bool {
    extract_payment_required(result).is_some()
}

/// Extracts V2 [`PaymentRequired`] from a JSON-RPC payment challenge.
///
/// TS `isPaymentRequiredError`: code `402` uses `data`; `-32042` uses `data` or `data.x402`.
#[must_use]
pub fn extract_payment_required_from_rpc(err: &McpCallError) -> Option<PaymentRequired> {
    let McpCallError::Rpc { code, data, .. } = err else {
        return None;
    };
    let data = data.as_ref()?;
    match *code {
        MCP_PAYMENT_REQUIRED_CODE => payment_required_from_value(data),
        JSONRPC_PAYMENT_REQUIRED_CODE => payment_required_from_value(data)
            .or_else(|| data.get("x402").and_then(payment_required_from_value)),
        _ => None,
    }
}

/// True when a JSON-RPC error is an x402 payment-required challenge.
#[must_use]
pub fn is_payment_required_rpc(err: &McpCallError) -> bool {
    extract_payment_required_from_rpc(err).is_some()
}

#[cfg(test)]
mod tests {
    use r402_core::wire::{PaymentRequirements, ResourceInfo};

    use super::*;

    fn sample_required() -> PaymentRequired {
        let resource = ResourceInfo::new("mcp://tool/demo");
        let req = PaymentRequirements::new(
            "exact".into(),
            "eip155:84532".parse().unwrap(),
            "10000".into(),
            "0xpay".into(),
            "0xasset".into(),
            60,
        );
        PaymentRequired::new(resource)
            .with_error("Payment required")
            .with_accepts(vec![req])
    }

    #[test]
    fn payment_required_sets_structured_and_text() {
        let pr = sample_required();
        let result = payment_required_tool_result(&pr);
        assert_eq!(result.is_error, Some(true));
        let sc = result.structured_content.as_ref().unwrap();
        let text = result
            .content
            .first()
            .and_then(ContentBlock::as_text)
            .unwrap();
        let from_text: Value = serde_json::from_str(&text.text).unwrap();
        assert_eq!(sc, &from_text);
        let again = extract_payment_required(&result).unwrap();
        assert_eq!(
            again.accepts.first().map(|a| a.amount.as_str()),
            Some("10000")
        );
    }

    #[test]
    fn settlement_failed_uses_dual_format() {
        let pr = sample_required();
        let result = settlement_failed_tool_result(
            &pr.accepts,
            &pr.resource,
            &Extensions::new(),
            "Settlement failed",
        );
        assert!(extract_payment_required(&result).is_some());
    }

    #[test]
    fn rejects_v1_shaped_payment_required() {
        let result = CallToolResult::structured_error(serde_json::json!({
            "x402Version": 1,
            "accepts": [{"scheme":"exact","network":"eip155:1","amount":"1","payTo":"0xa","asset":"0xb","maxTimeoutSeconds":60}]
        }));
        assert!(extract_payment_required(&result).is_none());
    }

    #[test]
    fn tool_url_default_and_custom() {
        assert_eq!(
            create_tool_resource_url("weather", None),
            "mcp://tool/weather"
        );
        assert_eq!(
            create_tool_resource_url("weather", Some("https://x/y")),
            "https://x/y"
        );
    }

    #[test]
    fn attach_and_extract_payment_meta() {
        let payload: McpPaymentPayload = serde_json::from_value(serde_json::json!({
            "x402Version": 2,
            "accepted": {
                "scheme": "exact",
                "network": "eip155:1",
                "amount": "1",
                "payTo": "0xa",
                "asset": "0xb",
                "maxTimeoutSeconds": 60
            },
            "payload": {"sig": "0x"}
        }))
        .unwrap();
        let params = CallToolRequestParams::new("demo");
        let params = attach_payment_to_params(params, &payload);
        let extracted = extract_payment_from_params(&params).unwrap();
        assert_eq!(extracted.accepted.scheme.as_str(), "exact");
    }

    #[test]
    fn null_payload_body_rejected() {
        use rmcp::model::RequestParamsMeta;

        let mut params = CallToolRequestParams::new("demo");
        let mut meta = RequestMetaObject::new();
        meta.insert(
            MCP_PAYMENT_META_KEY.to_owned(),
            serde_json::json!({
                "x402Version": 2,
                "accepted": {
                    "scheme": "exact",
                    "network": "eip155:1",
                    "amount": "1",
                    "payTo": "0xa",
                    "asset": "0xb",
                    "maxTimeoutSeconds": 60
                },
                "payload": null
            }),
        );
        params.set_meta(meta);
        assert!(extract_payment_from_params(&params).is_none());
    }

    #[test]
    fn attach_and_extract_settle_response() {
        let settle = SettleResponse::Success {
            payer: "0xpayer".into(),
            transaction: "0xtx".into(),
            network: "eip155:1".into(),
            amount: Some("1".into()),
            extensions: Extensions::new(),
            extra: None,
        };
        let result = CallToolResult::success(vec![ContentBlock::text("ok")]);
        let result = attach_settle_response(result, &settle);
        let extracted = extract_settle_response(&result).unwrap();
        assert!(extracted.is_success());
    }

    #[cfg(feature = "client")]
    mod rpc {
        use rmcp::ServiceError;
        use rmcp::model::{ErrorCode, ErrorData};
        use serde_json::json;

        use super::*;
        use crate::error::mcp_call_error_from_rmcp;

        #[test]
        fn jsonrpc_32042_direct_data_is_payment_required() {
            let data = serde_json::to_value(sample_required()).unwrap();
            let err = mcp_call_error_from_rmcp(ServiceError::McpError(ErrorData {
                code: ErrorCode(JSONRPC_PAYMENT_REQUIRED_CODE),
                message: "Payment Required".into(),
                data: Some(data),
            }));
            assert!(
                is_payment_required_rpc(&err),
                "direct -32042 data should parse"
            );
            let pr = extract_payment_required_from_rpc(&err).unwrap();
            assert_eq!(
                pr.accepts.first().map(|a| a.amount.as_str()),
                Some("10000"),
                "extracted amount"
            );
        }

        #[test]
        fn jsonrpc_32042_namespaced_x402_is_payment_required() {
            let data = json!({
                "challenges": [{"method": "tempo"}],
                "x402": sample_required(),
            });
            let err = mcp_call_error_from_rmcp(ServiceError::McpError(ErrorData {
                code: ErrorCode(JSONRPC_PAYMENT_REQUIRED_CODE),
                message: "Payment Required".into(),
                data: Some(data),
            }));
            assert!(
                is_payment_required_rpc(&err),
                "namespaced -32042 data.x402 should parse"
            );
            let pr = extract_payment_required_from_rpc(&err).unwrap();
            assert_eq!(
                pr.accepts.first().map(|a| a.amount.as_str()),
                Some("10000"),
                "extracted namespaced amount"
            );
        }

        #[test]
        fn jsonrpc_402_direct_data_is_payment_required() {
            let data = serde_json::to_value(sample_required()).unwrap();
            let err = mcp_call_error_from_rmcp(ServiceError::McpError(ErrorData {
                code: ErrorCode(MCP_PAYMENT_REQUIRED_CODE),
                message: "Payment Required".into(),
                data: Some(data),
            }));
            assert!(
                is_payment_required_rpc(&err),
                "legacy 402 data should parse"
            );
        }

        #[test]
        fn jsonrpc_402_does_not_read_namespaced_x402() {
            let data = json!({
                "challenges": [{"method": "tempo"}],
                "x402": sample_required(),
            });
            let err = mcp_call_error_from_rmcp(ServiceError::McpError(ErrorData {
                code: ErrorCode(MCP_PAYMENT_REQUIRED_CODE),
                message: "Payment Required".into(),
                data: Some(data),
            }));
            assert!(
                !is_payment_required_rpc(&err),
                "402 must not read data.x402"
            );
        }

        #[test]
        fn jsonrpc_32042_without_payment_required_is_not_a_challenge() {
            let err = mcp_call_error_from_rmcp(ServiceError::McpError(ErrorData {
                code: ErrorCode(JSONRPC_PAYMENT_REQUIRED_CODE),
                message: "Payment Required".into(),
                data: Some(json!({"unrelated": true})),
            }));
            assert!(
                !is_payment_required_rpc(&err),
                "-32042 without PaymentRequired is not a challenge"
            );
            match err {
                McpCallError::Rpc { code, .. } => {
                    assert_eq!(
                        code, JSONRPC_PAYMENT_REQUIRED_CODE,
                        "adapter keeps RPC code"
                    );
                }
                McpCallError::Transport(msg) => panic!("expected Rpc, got Transport({msg})"),
            }
        }

        #[test]
        fn transport_service_error_is_not_a_challenge() {
            let err = mcp_call_error_from_rmcp(ServiceError::TransportClosed);
            assert!(
                !is_payment_required_rpc(&err),
                "transport errors are not payment challenges"
            );
            match err {
                McpCallError::Transport(_) => {}
                McpCallError::Rpc { code, message, .. } => {
                    panic!("expected Transport, got Rpc {{ code: {code}, message: {message} }}")
                }
            }
        }
    }
}
