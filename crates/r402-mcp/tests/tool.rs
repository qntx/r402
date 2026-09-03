#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::items_after_statements,
    clippy::panic,
    clippy::unnecessary_literal_bound,
    clippy::unwrap_used,
    reason = "idiomatic test-code patterns"
)]

//! Dual-format tool encoding and buyer auto-pay.

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use r402_client::{PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use r402_mcp::{
    JSONRPC_PAYMENT_REQUIRED_CODE, MCP_PAYMENT_META_KEY, MCP_PAYMENT_REQUIRED_CODE,
    MCP_PAYMENT_RESPONSE_META_KEY, McpCallError, McpClientError, McpPaymentPayload, McpToolCaller,
    X402McpClient, X402McpClientOptions, attach_payment_to_params, attach_settle_response,
    create_tool_resource_url, extract_payment_from_params, extract_payment_required,
    extract_payment_required_from_rpc, extract_settle_response, is_payment_required_rpc,
    mcp_call_error_from_rmcp, payment_required_tool_result, settlement_failed_tool_result,
};
use r402_protocol::error::ClientError;
use r402_protocol::payment::{
    Base64Bytes, Extensions, PaymentRequired, PaymentRequirements, ResourceInfo, SettleResponse,
};
use r402_protocol::{ChainId, SchemeId};
use rmcp::ServiceError;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, ErrorCode, ErrorData, RequestParamsMeta,
};
use serde_json::{Value, json};

fn sample_required() -> PaymentRequired {
    let network = "eip155:1".parse().expect("fixture network");
    let resource = ResourceInfo::new("mcp://tool/demo");
    let req = PaymentRequirements::new(
        "exact".into(),
        network,
        "10000".into(),
        "0xa".into(),
        "0xb".into(),
        60,
    );
    PaymentRequired::new(resource).with_accepts(vec![req])
}

#[test]
fn constants_match_foundation() {
    assert_eq!(MCP_PAYMENT_REQUIRED_CODE, 402);
    assert_eq!(JSONRPC_PAYMENT_REQUIRED_CODE, -32042);
    assert_eq!(MCP_PAYMENT_META_KEY, "x402/payment");
    assert_eq!(MCP_PAYMENT_RESPONSE_META_KEY, "x402/payment-response");
}

#[test]
fn payment_required_sets_structured_and_text() {
    let required = sample_required();
    let result = payment_required_tool_result(&required);
    assert_eq!(result.is_error, Some(true));
    let structured = result.structured_content.as_ref().unwrap();
    let text = result
        .content
        .first()
        .and_then(ContentBlock::as_text)
        .unwrap();
    let from_text: Value = serde_json::from_str(&text.text).unwrap();
    assert_eq!(structured, &from_text);
    let again = extract_payment_required(&result).unwrap();
    assert_eq!(
        again.accepts.first().map(|accept| accept.amount.as_str()),
        Some("10000")
    );
}

#[test]
fn settlement_failed_uses_dual_format() {
    let result = settlement_failed_tool_result(&sample_required());
    assert!(extract_payment_required(&result).is_some());
}

#[test]
fn rejects_v1_shaped_payment_required() {
    let result = CallToolResult::structured_error(json!({
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
    let payload: McpPaymentPayload = serde_json::from_value(json!({
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
    let params = attach_payment_to_params(CallToolRequestParams::new("demo"), &payload);
    let extracted = extract_payment_from_params(&params).unwrap();
    assert_eq!(extracted.accepted.scheme.as_str(), "exact");
}

#[test]
fn null_payload_body_rejected() {
    let mut params = CallToolRequestParams::new("demo");
    let mut meta = rmcp::model::RequestMetaObject::new();
    meta.insert(
        MCP_PAYMENT_META_KEY.to_owned(),
        json!({
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
        payer: Some("0xpayer".into()),
        transaction: "0xtx".into(),
        network: "eip155:1".into(),
        amount: Some("1".into()),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    };
    let result = attach_settle_response(
        CallToolResult::success(vec![ContentBlock::text("ok")]),
        &settle,
    );
    assert!(extract_settle_response(&result).unwrap().is_success());
}

fn rpc_err(code: i32, data: Value) -> McpCallError {
    mcp_call_error_from_rmcp(ServiceError::McpError(ErrorData {
        code: ErrorCode(code),
        message: "Payment Required".into(),
        data: Some(data),
    }))
}

#[test]
fn jsonrpc_32042_direct_and_namespaced() {
    let data = serde_json::to_value(sample_required()).unwrap();
    assert!(is_payment_required_rpc(&rpc_err(
        JSONRPC_PAYMENT_REQUIRED_CODE,
        data
    )));
    let namespaced = json!({
        "challenges": [{"method": "tempo"}],
        "x402": sample_required(),
    });
    let pr = extract_payment_required_from_rpc(&rpc_err(JSONRPC_PAYMENT_REQUIRED_CODE, namespaced))
        .unwrap();
    assert_eq!(
        pr.accepts.first().map(|accept| accept.amount.as_str()),
        Some("10000")
    );
}

#[test]
fn jsonrpc_402_direct_not_namespaced() {
    let data = serde_json::to_value(sample_required()).unwrap();
    assert!(is_payment_required_rpc(&rpc_err(
        MCP_PAYMENT_REQUIRED_CODE,
        data
    )));
    let namespaced = json!({
        "challenges": [{"method": "tempo"}],
        "x402": sample_required(),
    });
    assert!(!is_payment_required_rpc(&rpc_err(
        MCP_PAYMENT_REQUIRED_CODE,
        namespaced
    )));
}

#[test]
fn jsonrpc_32042_without_payment_required_is_not_a_challenge() {
    let err = rpc_err(JSONRPC_PAYMENT_REQUIRED_CODE, json!({"unrelated": true}));
    assert!(!is_payment_required_rpc(&err));
}

#[test]
fn transport_service_error_is_not_a_challenge() {
    let err = mcp_call_error_from_rmcp(ServiceError::TransportClosed);
    assert!(!is_payment_required_rpc(&err));
    match err {
        McpCallError::Transport(_) => {}
        McpCallError::Rpc { code, message, .. } => {
            panic!("expected Transport, got Rpc {{ code: {code}, message: {message} }}")
        }
    }
}

struct MockCaller {
    calls: Mutex<u8>,
}

impl McpToolCaller for MockCaller {
    fn call_tool(
        &self,
        params: CallToolRequestParams,
    ) -> impl Future<Output = Result<CallToolResult, McpCallError>> + Send {
        std::future::ready(self.handle(&params))
    }
}

impl MockCaller {
    fn handle(&self, params: &CallToolRequestParams) -> Result<CallToolResult, McpCallError> {
        let unpaid = params.meta.is_none();
        let mut n = self
            .calls
            .lock()
            .map_err(|e| McpCallError::Transport(e.to_string()))?;
        *n = n.saturating_add(1);
        drop(n);
        if unpaid {
            Ok(payment_required_tool_result(&sample_required()))
        } else {
            Ok(CallToolResult::success(vec![ContentBlock::text("ok")]))
        }
    }
}

struct RpcCaller {
    calls: Mutex<u8>,
    data: Value,
}

impl McpToolCaller for RpcCaller {
    fn call_tool(
        &self,
        params: CallToolRequestParams,
    ) -> impl Future<Output = Result<CallToolResult, McpCallError>> + Send {
        std::future::ready(self.handle(&params))
    }
}

impl RpcCaller {
    fn handle(&self, params: &CallToolRequestParams) -> Result<CallToolResult, McpCallError> {
        let unpaid = params.meta.is_none();
        let mut n = self
            .calls
            .lock()
            .map_err(|e| McpCallError::Transport(e.to_string()))?;
        *n = n.saturating_add(1);
        drop(n);
        if unpaid {
            Err(rpc_err(JSONRPC_PAYMENT_REQUIRED_CODE, self.data.clone()))
        } else {
            Ok(CallToolResult::success(vec![ContentBlock::text("ok")]))
        }
    }
}

struct FixtureSigner(PaymentRequirements);

impl PaymentCandidateSigner for FixtureSigner {
    fn sign_payment<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + 'a>> {
        Box::pin(async {
            let payload = McpPaymentPayload::new(self.0.clone(), json!({"s": 1}));
            let bytes = serde_json::to_vec(&payload)
                .map_err(|err| ClientError::Signing(err.to_string()))?;
            Ok(Base64Bytes::encode(&bytes).to_string())
        })
    }
}

struct FixtureScheme;

impl SchemeId for FixtureScheme {
    fn namespace(&self) -> &str {
        "eip155"
    }

    fn scheme(&self) -> &str {
        "exact"
    }
}

impl SchemeClient for FixtureScheme {
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        payment_required
            .accepts
            .iter()
            .cloned()
            .map(|req| PaymentCandidate {
                chain_id: req.network.clone(),
                asset: req.asset.clone(),
                amount: req.amount.clone(),
                scheme: req.scheme.clone(),
                pay_to: req.pay_to.clone(),
                signer: Box::new(FixtureSigner(req.clone())),
                requirements: req,
            })
            .collect()
    }

    fn find_default_asset(
        &self,
        _asset: &str,
        _network: &ChainId,
    ) -> Option<r402_client::DefaultAssetInfo> {
        None
    }
}

fn paying_client<C: McpToolCaller>(caller: C) -> X402McpClient<C> {
    X402McpClient::new(caller)
        .register(FixtureScheme)
        .disable_spend_controls()
}

#[tokio::test]
async fn auto_pays_on_second_call() {
    let out = paying_client(MockCaller {
        calls: Mutex::new(0),
    })
    .call_tool("demo", None)
    .await
    .unwrap();
    assert!(out.payment_made, "tool-result 402 should auto-pay");
    assert!(
        !out.result.is_error.unwrap_or(false),
        "paid retry should succeed"
    );
}

#[tokio::test]
async fn auto_payment_disabled_returns_402() {
    let err = paying_client(MockCaller {
        calls: Mutex::new(0),
    })
    .with_options(X402McpClientOptions {
        auto_payment: false,
    })
    .call_tool("demo", None)
    .await
    .unwrap_err();
    match err {
        McpClientError::PaymentRequired(err) => {
            assert_eq!(err.code, 402, "user-facing error stays 402");
        }
        other => panic!("expected PaymentRequired, got {other:?}"),
    }
}

#[tokio::test]
async fn auto_pays_on_jsonrpc_32042() {
    let data = serde_json::to_value(sample_required()).unwrap();
    let out = paying_client(RpcCaller {
        calls: Mutex::new(0),
        data,
    })
    .call_tool("demo", None)
    .await
    .unwrap();
    assert!(out.payment_made, "-32042 direct data should auto-pay");
}

#[tokio::test]
async fn non_payment_rpc_is_transport() {
    let err = paying_client(RpcCaller {
        calls: Mutex::new(0),
        data: json!({"unrelated": true}),
    })
    .call_tool("demo", None)
    .await
    .unwrap_err();
    match err {
        McpClientError::Transport(_) => {}
        other => panic!("expected Transport, got {other:?}"),
    }
}
