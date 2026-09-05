//! In-process upto-scheme tests. No HTTP E2E.
//!
//! Covering e2e (`anvil_erc20_*`) spawns Foundry `anvil` (PATH or
//! `~/.foundry/bin/anvil`). Skips if the binary is missing or
//! `R402_ANVIL=0`. Wiremock tests do not need Anvil.

#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::doc_markdown,
    clippy::excessive_nesting,
    clippy::expect_used,
    clippy::if_same_then_else,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used,
    reason = "idiomatic test-code patterns"
)]

use std::str::FromStr;
use std::sync::{Arc, Mutex};

use alloy_network::{EthereumWallet, TransactionBuilder};
use alloy_primitives::{Address, U256, address, hex};
use alloy_provider::Provider;
use alloy_signer_local::PrivateKeySigner;
use r402_client::SchemeClient;
use r402_evm::chain::{ChecksummedAddress, Eip155ChainProvider, Eip155ChainReference};
use r402_evm::eip2612::EIP2612_GAS_SPONSORING_KEY;
use r402_evm::erc20_approval::ERC20_APPROVAL_GAS_SPONSORING_KEY;
use r402_evm::upto::payload::UptoPaymentRequirementsExtra;
use r402_evm::{
    Eip155Upto, Eip155UptoClient, Eip155UptoFacilitator, PERMIT2_ADDRESS, USDC,
    X402_UPTO_PERMIT2_PROXY,
};
use r402_facilitator::Facilitator;
use r402_protocol::error::{AsPaymentProblem, ErrorReason, FacilitatorError, VerificationError};
use r402_protocol::payment::{
    PaymentRequired, ResourceInfo, SettleRequest, SettleResponse, VerifyRequest, VerifyResponse,
};
use r402_protocol::scheme::SchemeId;
use r402_server::{PaymentFlowName, SchemeNetworkServer};
use url::Url;

/// Anvil account 0. Local signing only; never sent to a live chain.
const ANVIL_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

fn wallet() -> (PrivateKeySigner, EthereumWallet) {
    let signer = PrivateKeySigner::from_str(ANVIL_KEY).expect("anvil key");
    let wallet = EthereumWallet::from(signer.clone());
    (signer, wallet)
}

fn dummy_provider() -> Eip155ChainProvider {
    let (_signer, wallet) = wallet();
    let url = Url::parse("https://example.invalid").expect("url");
    Eip155ChainProvider::new(
        Eip155ChainReference::new(8453),
        wallet,
        &[(url, None)],
        true,
        false,
        1,
    )
    .expect("provider constructs without connecting")
}

const fn pay_to() -> Address {
    address!("0x1111111111111111111111111111111111111111")
}

fn facilitator_addr() -> Address {
    let (signer, _) = wallet();
    signer.address()
}

#[test]
fn scheme_id_and_payment_flows() {
    let scheme = Eip155Upto;
    assert_eq!(scheme.namespace(), "eip155");
    assert_eq!(SchemeNetworkServer::scheme(&scheme), "upto");
    assert_eq!(scheme.default_asset_transfer_method(), "permit2");
    let expected = r402_server::PaymentFlowConfig::authorization_only();
    assert_eq!(scheme.payment_flows().get("permit2"), Some(&expected));
    assert_eq!(expected.default, PaymentFlowName::Authorization);
    assert_eq!(expected.supported, vec![PaymentFlowName::Authorization]);
}

#[test]
fn proxy_address_is_canonical() {
    assert_eq!(
        X402_UPTO_PERMIT2_PROXY.to_checksum(None),
        "0x4020A4f3b7b90ccA423B9fabCc0CE57C6C240002"
    );
}

#[test]
fn price_tag_is_permit2_max_without_facilitator() {
    let tag = Eip155Upto::price_tag(pay_to(), USDC::base().amount(5_000_000u64));
    assert_eq!(tag.requirements.scheme, "upto");
    assert_eq!(tag.requirements.network.to_string(), "eip155:8453");
    assert_eq!(tag.requirements.amount, "5000000");
    let extra = tag.requirements.extra.unwrap();
    assert_eq!(
        extra
            .get("assetTransferMethod")
            .and_then(serde_json::Value::as_str),
        Some("permit2")
    );
    assert!(extra.get("facilitatorAddress").is_none());
}

#[test]
fn price_tag_with_facilitator_pins_address() {
    let tag = Eip155Upto::price_tag_with_facilitator(
        pay_to(),
        USDC::base().amount(1_000_000u64),
        facilitator_addr(),
    );
    let extra: UptoPaymentRequirementsExtra =
        serde_json::from_value(tag.requirements.extra.unwrap()).unwrap();
    assert_eq!(extra.facilitator_address.0, facilitator_addr());
}

fn try_new_question_mark() -> Result<(), FacilitatorError> {
    let _fac = Eip155UptoFacilitator::try_new(dummy_provider())?;
    Ok(())
}

#[test]
fn try_new_question_mark_compiles() {
    try_new_question_mark().expect("try_new is currently infallible");
}

#[tokio::test]
async fn try_new_supported_is_upto_on_provider_chain() {
    let fac = Eip155UptoFacilitator::try_new(dummy_provider()).expect("try_new");
    let supported = fac.supported().await.expect("supported");
    let kind = supported.kinds.first().expect("one kind");
    assert_eq!(kind.scheme, "upto");
    assert_eq!(kind.network, "eip155:8453");
    assert_eq!(kind.x402_version, 2);
    assert!(
        !supported.signers.is_empty(),
        "try_new provider must advertise signers"
    );
    let extra = kind.extra.as_ref().expect("upto kind extra");
    assert_eq!(
        extra
            .get("assetTransferMethod")
            .and_then(serde_json::Value::as_str),
        Some("permit2")
    );
    assert!(extra.get("facilitatorAddress").is_some());
    assert!(
        supported
            .extensions
            .iter()
            .any(|e| e.as_str() == EIP2612_GAS_SPONSORING_KEY)
    );
    assert!(
        supported
            .extensions
            .iter()
            .any(|e| e.as_str() == ERC20_APPROVAL_GAS_SPONSORING_KEY)
    );
}

#[tokio::test]
async fn verify_malformed_payload_is_invalid_format() {
    let fac = Eip155UptoFacilitator::try_new(dummy_provider()).expect("try_new");
    let err = fac
        .verify(VerifyRequest::from(serde_json::json!({})))
        .await
        .expect_err("empty JSON is not a verify request");
    assert!(
        matches!(
            err,
            FacilitatorError::Verification(VerificationError::InvalidFormat(_))
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn client_accepts_and_signs_permit2_locally() {
    let (signer, _) = wallet();
    let client = Eip155UptoClient::new(Arc::new(signer));
    let tag = Eip155Upto::price_tag_with_facilitator(
        pay_to(),
        USDC::base().amount(1_000_000u64),
        facilitator_addr(),
    );
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()]);
    let candidates = client.accept(&required);
    assert_eq!(candidates.len(), 1, "one upto accept");
    let b64 = candidates
        .first()
        .expect("one upto accept")
        .sign()
        .await
        .expect("local Permit2 sign");
    let json = r402_protocol::payment::Base64Bytes(b64.into_bytes())
        .decode()
        .expect("b64");
    let payload: r402_evm::upto::payload::v2::PaymentPayload =
        serde_json::from_slice(&json).expect("payload");
    assert_eq!(
        payload.payload.permit2_authorization.spender,
        X402_UPTO_PERMIT2_PROXY
    );
    assert_eq!(
        payload.payload.permit2_authorization.witness.facilitator,
        facilitator_addr()
    );
    assert_eq!(payload.accepted.network.to_string(), "eip155:8453");
}

#[tokio::test]
async fn settle_rejects_actual_above_signed_max() {
    let (signer, _) = wallet();
    let client = Eip155UptoClient::new(Arc::new(signer));
    let tag = Eip155Upto::price_tag_with_facilitator(
        pay_to(),
        USDC::base().amount(5_000_000u64),
        facilitator_addr(),
    );
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()]);
    let b64 = client
        .accept(&required)
        .first()
        .expect("candidate")
        .sign()
        .await
        .expect("sign");
    let json = r402_protocol::payment::Base64Bytes(b64.into_bytes())
        .decode()
        .expect("b64");
    let payload: serde_json::Value = serde_json::from_slice(&json).expect("json");

    let mut requirements = tag.requirements.clone();
    requirements.amount = "6000000".into();
    let settle = serde_json::json!({
        "x402Version": 2,
        "paymentPayload": payload,
        "paymentRequirements": requirements,
    });
    let fac = Eip155UptoFacilitator::try_new(dummy_provider()).expect("try_new");
    let err = fac
        .settle(SettleRequest::from(settle))
        .await
        .expect_err("actual above max");
    assert!(
        matches!(
            err,
            FacilitatorError::Verification(
                VerificationError::SettlementAmountExceedsPermitted { .. }
            )
        ),
        "got {err:?}"
    );
}

#[test]
fn permit2_allowance_required_is_412_reason() {
    let problem = VerificationError::Permit2AllowanceRequired.as_payment_problem();
    assert_eq!(problem.reason(), ErrorReason::Permit2AllowanceRequired);
    assert_eq!(problem.reason().as_str(), "permit2_allowance_required");
}

fn provider_at(rpc: &str) -> Eip155ChainProvider {
    let (_signer, wallet) = wallet();
    let url = Url::parse(rpc).expect("url");
    Eip155ChainProvider::new(
        Eip155ChainReference::new(8453),
        wallet,
        &[(url, None)],
        true,
        false,
        1,
    )
    .expect("provider")
}

fn checksum(addr: Address) -> String {
    addr.to_checksum(None)
}

#[derive(Clone, Default)]
struct RpcLog {
    methods: Arc<Mutex<Vec<String>>>,
    call_targets: Arc<Mutex<Vec<Address>>>,
}

impl RpcLog {
    fn record_method(&self, method: &str) {
        self.methods.lock().unwrap().push(method.to_owned());
    }

    fn record_call_to(&self, to: Address) {
        self.call_targets.lock().unwrap().push(to);
    }

    fn methods(&self) -> Vec<String> {
        self.methods.lock().unwrap().clone()
    }

    fn called(&self, to: Address) -> bool {
        self.call_targets.lock().unwrap().contains(&to)
    }
}

#[derive(Clone)]
struct RpcScript {
    asset: Address,
    #[allow(dead_code, reason = "kept for RPC script identity")]
    payer: Address,
    allowance: Option<U256>,
    balance: U256,
    native_balance: U256,
    tx_count: u64,
    base_fee: u64,
    revert_settle: bool,
    simulate_status: bool,
    simulate_missing: bool,
    log: RpcLog,
}

fn rpc_script(asset: Address, payer: Address, allowance: Option<U256>, balance: U256) -> RpcScript {
    RpcScript {
        asset,
        payer,
        allowance,
        balance,
        native_balance: U256::from(10).pow(U256::from(18)),
        tx_count: 0,
        base_fee: 1,
        revert_settle: true,
        simulate_status: true,
        simulate_missing: false,
        log: RpcLog::default(),
    }
}

fn latest_block_json(base_fee: u64) -> serde_json::Value {
    let zero32 = format!("0x{}", "00".repeat(32));
    let bloom = format!("0x{}", "00".repeat(256));
    serde_json::json!({
        "hash": zero32,
        "parentHash": zero32,
        "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
        "miner": "0x0000000000000000000000000000000000000000",
        "stateRoot": zero32,
        "transactionsRoot": zero32,
        "receiptsRoot": zero32,
        "logsBloom": bloom,
        "difficulty": "0x0",
        "number": "0x1",
        "gasLimit": "0x1c9c380",
        "gasUsed": "0x0",
        "timestamp": "0x1",
        "extraData": "0x",
        "mixHash": zero32,
        "nonce": "0x0000000000000000",
        "baseFeePerGas": format!("0x{base_fee:x}"),
        "uncles": [],
        "transactions": [],
        "size": "0x0",
        "totalDifficulty": "0x0"
    })
}

#[allow(
    clippy::excessive_nesting,
    clippy::get_first,
    clippy::option_if_let_else,
    reason = "JSON-RPC dispatch on method and selector"
)]
impl wiremock::Respond for RpcScript {
    fn respond(&self, request: &wiremock::Request) -> wiremock::ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap_or_default();
        let id = body
            .get("id")
            .cloned()
            .unwrap_or_else(|| serde_json::json!(1));
        let method = body
            .get("method")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let ok = |result: serde_json::Value| {
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result
            }))
        };
        self.log.record_method(method);
        match method {
            "eth_getCode" => {
                let addr = body
                    .get("params")
                    .and_then(|p| p.get(0))
                    .and_then(serde_json::Value::as_str)
                    .and_then(|s| s.parse::<Address>().ok())
                    .unwrap_or(Address::ZERO);
                let code = if addr == self.asset {
                    "0x6080604052"
                } else {
                    "0x"
                };
                ok(serde_json::json!(code))
            }
            "eth_call" => {
                let call = body.get("params").and_then(|p| p.get(0));
                let to = call
                    .and_then(|c| c.get("to"))
                    .and_then(serde_json::Value::as_str)
                    .and_then(|s| s.parse::<Address>().ok())
                    .unwrap_or(Address::ZERO);
                self.log.record_call_to(to);
                let data = call
                    .and_then(|c| c.get("data").or_else(|| c.get("input")))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("0x");
                let selector = data.get(2..10).unwrap_or("").to_ascii_lowercase();
                if selector == "dd62ed3e" {
                    return match self.allowance {
                        Some(value) => ok(serde_json::json!(format!("0x{value:064x}"))),
                        None => {
                            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "error": { "code": -32000, "message": "allowance rpc failed" }
                            }))
                        }
                    };
                }
                if selector == "70a08231" {
                    return ok(serde_json::json!(format!("0x{:064x}", self.balance)));
                }
                if self.revert_settle && to == X402_UPTO_PERMIT2_PROXY {
                    return wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": 3, "message": "execution reverted" }
                    }));
                }
                ok(serde_json::json!(
                    "0x0000000000000000000000000000000000000000000000000000000000000000"
                ))
            }
            "eth_getBalance" => ok(serde_json::json!(format!("0x{:x}", self.native_balance))),
            "eth_getTransactionCount" => ok(serde_json::json!(format!("0x{:x}", self.tx_count))),
            "eth_getBlockByNumber" => ok(latest_block_json(self.base_fee)),
            "eth_simulateV1" => {
                if self.simulate_missing {
                    return wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": "the method eth_simulateV1 does not exist/is not available" }
                    }));
                }
                let call_count = body
                    .get("params")
                    .and_then(|p| p.get(0))
                    .and_then(|p| p.get("blockStateCalls"))
                    .and_then(|b| b.get(0))
                    .and_then(|b| b.get("calls"))
                    .and_then(serde_json::Value::as_array)
                    .map_or(2, Vec::len);
                let mut block = latest_block_json(self.base_fee);
                let status = if self.simulate_status { "0x1" } else { "0x0" };
                let calls = (0..call_count)
                    .map(|_| {
                        serde_json::json!({
                            "returnData": "0x",
                            "logs": [],
                            "gasUsed": "0x1",
                            "status": status
                        })
                    })
                    .collect::<Vec<_>>();
                block
                    .as_object_mut()
                    .expect("block object")
                    .insert("calls".into(), serde_json::Value::Array(calls));
                ok(serde_json::json!([block]))
            }
            _ => ok(serde_json::json!("0x")),
        }
    }
}

async fn mount_rpc(server: &wiremock::MockServer, script: RpcScript) {
    use wiremock::matchers::method;
    wiremock::Mock::given(method("POST"))
        .respond_with(script)
        .mount(server)
        .await;
}

async fn signed_upto_payload() -> (serde_json::Value, serde_json::Value, Address, Address) {
    let (signer, _) = wallet();
    let payer = signer.address();
    let client = Eip155UptoClient::new(Arc::new(signer));
    let tag = Eip155Upto::price_tag_with_facilitator(
        pay_to(),
        USDC::base().amount(1_000_000u64),
        facilitator_addr(),
    );
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()]);
    let b64 = client
        .accept(&required)
        .first()
        .expect("candidate")
        .sign()
        .await
        .expect("sign");
    let json = r402_protocol::payment::Base64Bytes(b64.into_bytes())
        .decode()
        .expect("b64");
    let payload: serde_json::Value = serde_json::from_slice(&json).expect("json");
    (
        payload,
        serde_json::to_value(&tag.requirements).expect("requirements"),
        payer,
        USDC::base().address,
    )
}

#[tokio::test]
async fn permit2_allowance_rpc_err_without_extensions_is_412() {
    let server = wiremock::MockServer::start().await;
    let (payload, requirements, payer, asset) = signed_upto_payload().await;
    mount_rpc(
        &server,
        RpcScript {
            revert_settle: false,
            ..rpc_script(asset, payer, None, U256::from(1_000_000_u64))
        },
    )
    .await;
    let fac = Eip155UptoFacilitator::try_new(provider_at(&server.uri())).expect("try_new");
    let err = fac
        .verify(VerifyRequest::from(serde_json::json!({
            "x402Version": 2,
            "paymentPayload": payload,
            "paymentRequirements": requirements,
        })))
        .await
        .expect_err("allowance rpc fail-closed");
    assert!(matches!(
        err,
        FacilitatorError::Verification(VerificationError::Permit2AllowanceRequired)
    ));
}

#[tokio::test]
async fn permit2_missing_erc20_key_low_allowance_is_412() {
    let server = wiremock::MockServer::start().await;
    let (payload, requirements, payer, asset) = signed_upto_payload().await;
    mount_rpc(
        &server,
        RpcScript {
            revert_settle: false,
            ..rpc_script(asset, payer, Some(U256::ZERO), U256::from(1_000_000_u64))
        },
    )
    .await;
    let fac = Eip155UptoFacilitator::try_new(provider_at(&server.uri())).expect("try_new");
    let err = fac
        .verify(VerifyRequest::from(serde_json::json!({
            "x402Version": 2,
            "paymentPayload": payload,
            "paymentRequirements": requirements,
        })))
        .await
        .expect_err("missing covering is 412");
    assert!(matches!(
        err,
        FacilitatorError::Verification(VerificationError::Permit2AllowanceRequired)
    ));
}

#[tokio::test]
async fn permit2_allowance_rpc_err_with_eip2612_continues() {
    let server = wiremock::MockServer::start().await;
    let (mut payload, requirements, payer, asset) = signed_upto_payload().await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    payload["extensions"] = serde_json::json!({
        "eip2612GasSponsoring": {
            "info": {
                "from": checksum(payer),
                "asset": checksum(asset),
                "spender": checksum(PERMIT2_ADDRESS),
                "amount": "1000000",
                "nonce": "0",
                "deadline": (now + 3600).to_string(),
                "signature": format!("0x{}", "11".repeat(65)),
                "version": "1"
            }
        }
    });
    mount_rpc(
        &server,
        RpcScript {
            revert_settle: false,
            ..rpc_script(asset, payer, None, U256::from(1_000_000_u64))
        },
    )
    .await;
    let fac = Eip155UptoFacilitator::try_new(provider_at(&server.uri())).expect("try_new");
    let resp = fac
        .verify(VerifyRequest::from(serde_json::json!({
            "x402Version": 2,
            "paymentPayload": payload,
            "paymentRequirements": requirements,
        })))
        .await
        .expect("eip2612 catch continues past 412");
    assert!(matches!(resp, VerifyResponse::Valid { .. }), "got {resp:?}");
}

fn sign_max_approve(signer: &PrivateKeySigner, token: Address) -> String {
    use alloy_consensus::{SignableTransaction, TxEip1559, TxEnvelope};
    use alloy_eips::eip2718::Encodable2718;
    use alloy_eips::eip2930::AccessList;
    use alloy_network::TxSignerSync;
    use alloy_primitives::TxKind;

    let mut calldata = vec![0x09, 0x5e, 0xa7, 0xb3];
    calldata.extend_from_slice(&[0u8; 12]);
    calldata.extend_from_slice(PERMIT2_ADDRESS.as_slice());
    calldata.extend_from_slice(&U256::MAX.to_be_bytes::<32>());
    let mut tx = TxEip1559 {
        chain_id: 8453,
        nonce: 0,
        gas_limit: 80_000,
        max_fee_per_gas: 1_000_000_000,
        max_priority_fee_per_gas: 1_000_000,
        to: TxKind::Call(token),
        value: U256::ZERO,
        access_list: AccessList::default(),
        input: calldata.into(),
    };
    let sig = signer.sign_transaction_sync(&mut tx).expect("sign");
    format!(
        "0x{}",
        hex::encode(TxEnvelope::Eip1559(tx.into_signed(sig)).encoded_2718())
    )
}

async fn signed_upto_with_erc20() -> (
    serde_json::Value,
    serde_json::Value,
    Address,
    Address,
    RpcLog,
) {
    let (signer, _) = wallet();
    let payer = signer.address();
    let asset = USDC::base().address;
    let client = Eip155UptoClient::new(Arc::new(signer.clone()));
    let tag = Eip155Upto::price_tag_with_facilitator(
        pay_to(),
        USDC::base().amount(1_000_000u64),
        facilitator_addr(),
    );
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()]);
    let b64 = client
        .accept(&required)
        .first()
        .expect("candidate")
        .sign()
        .await
        .expect("sign");
    let json = r402_protocol::payment::Base64Bytes(b64.into_bytes())
        .decode()
        .expect("b64");
    let mut payload: serde_json::Value = serde_json::from_slice(&json).expect("json");
    let signed = sign_max_approve(&signer, asset);
    payload["extensions"] = serde_json::json!({
        "erc20ApprovalGasSponsoring": {
            "info": {
                "from": checksum(payer),
                "asset": checksum(asset),
                "spender": checksum(PERMIT2_ADDRESS),
                "amount": U256::MAX.to_string(),
                "signedTransaction": signed,
                "version": "1"
            }
        }
    });
    (
        payload,
        serde_json::to_value(&tag.requirements).expect("requirements"),
        payer,
        asset,
        RpcLog::default(),
    )
}

#[tokio::test]
async fn permit2_erc20_stub_is_402_parse_failed() {
    let server = wiremock::MockServer::start().await;
    let (mut payload, requirements, payer, asset) = signed_upto_payload().await;
    payload["extensions"] = serde_json::json!({
        "erc20ApprovalGasSponsoring": {
            "info": {
                "from": checksum(payer),
                "asset": checksum(asset),
                "spender": checksum(PERMIT2_ADDRESS),
                "amount": U256::MAX.to_string(),
                "signedTransaction": "0xdead",
                "version": "1"
            }
        }
    });
    mount_rpc(
        &server,
        rpc_script(asset, payer, Some(U256::ZERO), U256::from(1_000_000_u64)),
    )
    .await;
    let fac = Eip155UptoFacilitator::try_new(provider_at(&server.uri())).expect("try_new");
    let err = fac
        .verify(VerifyRequest::from(serde_json::json!({
            "x402Version": 2,
            "paymentPayload": payload,
            "paymentRequirements": requirements,
        })))
        .await
        .expect_err("invalid envelope is 402");
    match err {
        FacilitatorError::Verification(e) => {
            assert_eq!(
                e.as_payment_problem().reason().as_str(),
                "erc20_approval_tx_parse_failed"
            );
        }
        other => panic!("expected verification, got {other:?}"),
    }
}

#[tokio::test]
async fn permit2_erc20_covering_skips_isolated_settle_and_calls_simulate() {
    let server = wiremock::MockServer::start().await;
    let (payload, requirements, payer, asset, _) = signed_upto_with_erc20().await;
    let script = RpcScript {
        revert_settle: true,
        simulate_status: true,
        ..rpc_script(asset, payer, Some(U256::ZERO), U256::from(1_000_000_u64))
    };
    let log = script.log.clone();
    mount_rpc(&server, script).await;
    let fac = Eip155UptoFacilitator::try_new(provider_at(&server.uri())).expect("try_new");
    let resp = fac
        .verify(VerifyRequest::from(serde_json::json!({
            "x402Version": 2,
            "paymentPayload": payload,
            "paymentRequirements": requirements,
        })))
        .await
        .expect("covering simulate succeeds");
    assert!(matches!(resp, VerifyResponse::Valid { .. }), "got {resp:?}");
    assert!(
        log.methods().iter().any(|m| m == "eth_simulateV1"),
        "covering must call eth_simulateV1, got {:?}",
        log.methods()
    );
    assert!(
        !log.called(X402_UPTO_PERMIT2_PROXY),
        "covering must not isolated eth_call settle"
    );
}

#[tokio::test]
async fn permit2_erc20_simulate_status_false_is_simulation_failed() {
    let server = wiremock::MockServer::start().await;
    let (payload, requirements, payer, asset, _) = signed_upto_with_erc20().await;
    mount_rpc(
        &server,
        RpcScript {
            revert_settle: true,
            simulate_status: false,
            ..rpc_script(asset, payer, Some(U256::ZERO), U256::from(1_000_000_u64))
        },
    )
    .await;
    let fac = Eip155UptoFacilitator::try_new(provider_at(&server.uri())).expect("try_new");
    let err = fac
        .verify(VerifyRequest::from(serde_json::json!({
            "x402Version": 2,
            "paymentPayload": payload,
            "paymentRequirements": requirements,
        })))
        .await
        .expect_err("status false");
    assert!(
        matches!(
            err,
            FacilitatorError::Verification(VerificationError::SimulationFailed(_))
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn permit2_erc20_missing_simulate_is_transport() {
    let server = wiremock::MockServer::start().await;
    let (payload, requirements, payer, asset, _) = signed_upto_with_erc20().await;
    mount_rpc(
        &server,
        RpcScript {
            revert_settle: true,
            simulate_missing: true,
            ..rpc_script(asset, payer, Some(U256::ZERO), U256::from(1_000_000_u64))
        },
    )
    .await;
    let fac = Eip155UptoFacilitator::try_new(provider_at(&server.uri())).expect("try_new");
    let err = fac
        .verify(VerifyRequest::from(serde_json::json!({
            "x402Version": 2,
            "paymentPayload": payload,
            "paymentRequirements": requirements,
        })))
        .await
        .expect_err("missing eth_simulateV1");
    assert!(err.is_transport(), "got {err:?}");
}

#[tokio::test]
async fn client_attaches_eip2612_when_advertised_and_allowance_low() {
    use std::future::Future;
    use std::pin::Pin;

    use r402_evm::Permit2Approver;
    use r402_evm::eip2612::EIP2612_GAS_SPONSORING_KEY;
    use r402_protocol::error::ClientError;
    use r402_protocol::payment::{ExtensionEntry, Extensions};

    struct LowAllowance;
    impl Permit2Approver for LowAllowance {
        fn check_permit2_allowance(
            &self,
            _token: Address,
            _owner: Address,
        ) -> Pin<Box<dyn Future<Output = Result<U256, ClientError>> + Send + '_>> {
            Box::pin(async { Ok(U256::ZERO) })
        }
        fn approve_permit2(
            &self,
            _token: Address,
            _owner: Address,
        ) -> Pin<Box<dyn Future<Output = Result<(), ClientError>> + Send + '_>> {
            Box::pin(async { Ok(()) })
        }
        fn supports_gas_sponsoring_rpc(&self) -> bool {
            true
        }
        fn eip2612_nonce(
            &self,
            _token: Address,
            _owner: Address,
        ) -> Pin<Box<dyn Future<Output = Result<U256, ClientError>> + Send + '_>> {
            Box::pin(async { Ok(U256::from(1u64)) })
        }
        fn transaction_count(
            &self,
            _owner: Address,
        ) -> Pin<Box<dyn Future<Output = Result<u64, ClientError>> + Send + '_>> {
            Box::pin(async { Ok(0) })
        }
        fn estimate_fees_per_gas(
            &self,
        ) -> Pin<Box<dyn Future<Output = Result<(u128, u128), ClientError>> + Send + '_>> {
            Box::pin(async { Ok((1_000_000_000, 100_000_000)) })
        }
    }

    let (signer, _) = wallet();
    let client = Eip155UptoClient::builder(signer)
        .approver(LowAllowance)
        .auto_approve(false)
        .build();
    let extra = UptoPaymentRequirementsExtra::new(ChecksummedAddress::from(facilitator_addr()))
        .with_token_eip712("USD Coin".into(), "2".into());
    let mut tag = Eip155Upto::price_tag_with_facilitator(
        pay_to(),
        USDC::base().amount(1_000_000u64),
        facilitator_addr(),
    );
    tag.requirements.extra = Some(serde_json::to_value(&extra).unwrap());
    let mut advertised = Extensions::new();
    advertised.insert(
        EIP2612_GAS_SPONSORING_KEY,
        ExtensionEntry::info(serde_json::json!({"version": "1"})),
    );
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()])
        .with_extensions(advertised);
    let b64 = client
        .accept(&required)
        .first()
        .expect("candidate")
        .sign()
        .await
        .expect("sign");
    let json = r402_protocol::payment::Base64Bytes(b64.into_bytes())
        .decode()
        .expect("b64");
    let payload: r402_evm::upto::payload::v2::PaymentPayload =
        serde_json::from_slice(&json).expect("payload");
    assert!(payload.extensions.get(EIP2612_GAS_SPONSORING_KEY).is_some());
}

/// Anvil account 1 — 0-ETH payer distinct from the facilitator hot wallet.
const ANVIL_KEY_1: &str = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
const TOKEN: Address = address!("0x00000000000000000000000000000000000000aa");
const ALWAYS_SUCCEED: [u8; 1] = [0x00];
const ALWAYS_REVERT: [u8; 3] = [0x5f, 0x5f, 0xfd];

struct AnvilGuard {
    child: std::process::Child,
}

impl Drop for AnvilGuard {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

fn anvil_e2e_enabled() -> bool {
    !matches!(std::env::var("R402_ANVIL"), Ok(value) if value == "0")
}

fn spawn_anvil() -> Option<(AnvilGuard, Url)> {
    if !anvil_e2e_enabled() {
        return None;
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
    let port = listener.local_addr().ok()?.port();
    drop(listener);
    let home_anvil = std::env::var("HOME")
        .ok()
        .map(|home| format!("{home}/.foundry/bin/anvil"));
    let anvil_ok = |bin: &str| {
        std::process::Command::new(bin)
            .arg("--version")
            .output()
            .is_ok()
    };
    let anvil = match home_anvil.as_deref() {
        _ if anvil_ok("anvil") => "anvil",
        Some(path) if anvil_ok(path) => path,
        _ => "anvil",
    };
    let child = std::process::Command::new(anvil)
        .args([
            "--port",
            &port.to_string(),
            "--chain-id",
            "8453",
            "--silent",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let url = Url::parse(&format!("http://127.0.0.1:{port}")).ok()?;
    Some((AnvilGuard { child }, url))
}

async fn wait_anvil(url: &Url) -> bool {
    for _ in 0..50 {
        if std::net::TcpStream::connect_timeout(
            &format!("127.0.0.1:{}", url.port().unwrap_or(8545))
                .parse()
                .expect("addr"),
            std::time::Duration::from_millis(100),
        )
        .is_ok()
        {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    false
}

fn payer_wallet() -> PrivateKeySigner {
    PrivateKeySigner::from_str(ANVIL_KEY_1).expect("anvil key 1")
}

fn provider_wallet_at(rpc: &str, key: &str) -> Eip155ChainProvider {
    let signer = PrivateKeySigner::from_str(key).expect("key");
    let wallet = EthereumWallet::from(signer);
    let url = Url::parse(rpc).expect("url");
    Eip155ChainProvider::new(
        Eip155ChainReference::new(8453),
        wallet,
        &[(url, None)],
        true,
        false,
        15,
    )
    .expect("provider")
}

async fn anvil_set_code(provider: &impl Provider, addr: Address, code: alloy_primitives::Bytes) {
    let _: serde_json::Value = provider
        .raw_request("anvil_setCode".into(), (addr, code))
        .await
        .expect("anvil_setCode");
}

async fn anvil_set_balance(provider: &impl Provider, addr: Address, value: U256) {
    let _: serde_json::Value = provider
        .raw_request("anvil_setBalance".into(), (addr, value))
        .await
        .expect("anvil_setBalance");
}

async fn mint_tokens(provider: &Eip155ChainProvider, token: Address, to: Address, amount: U256) {
    use alloy_primitives::Bytes;
    use alloy_sol_types::{SolCall, sol};
    use r402_evm::chain::{Eip155MetaTransactionProvider, MetaTransaction};

    sol! {
        function mint(address to, uint256 amount);
    }
    let calldata: Bytes = mintCall { to, amount }.abi_encode().into();
    Eip155MetaTransactionProvider::send_transaction(
        provider,
        MetaTransaction {
            to: token,
            calldata,
            confirmations: 1,
            from: None,
            value: U256::ZERO,
        },
    )
    .await
    .expect("mint");
}

async fn token_allowance(provider: &impl Provider, token: Address, owner: Address) -> U256 {
    use alloy_sol_types::{SolCall, sol};
    sol! {
        function allowance(address owner, address spender) external view returns (uint256);
    }
    let data = allowanceCall {
        owner,
        spender: PERMIT2_ADDRESS,
    }
    .abi_encode();
    let result = provider
        .call(
            alloy_rpc_types_eth::TransactionRequest::default()
                .with_to(token)
                .with_input(data),
        )
        .await
        .expect("allowance");
    U256::from_be_slice(result.as_ref())
}

async fn signed_upto_erc20_for(
    payer: &PrivateKeySigner,
    asset: Address,
    facilitator: Address,
) -> (serde_json::Value, serde_json::Value) {
    let client = Eip155UptoClient::new(Arc::new(payer.clone()));
    let tag = Eip155Upto::price_tag_with_facilitator(
        pay_to(),
        USDC::base().amount(1_000_000u64),
        facilitator,
    );
    let mut reqs = tag.requirements.clone();
    reqs.asset = checksum(asset).into();
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![reqs.clone()]);
    let b64 = client
        .accept(&required)
        .first()
        .expect("candidate")
        .sign()
        .await
        .expect("sign");
    let json = r402_protocol::payment::Base64Bytes(b64.into_bytes())
        .decode()
        .expect("b64");
    let mut payload: serde_json::Value = serde_json::from_slice(&json).expect("json");
    payload["accepted"]["asset"] = serde_json::json!(checksum(asset));
    payload["payload"]["permit2Authorization"]["permitted"]["token"] =
        serde_json::json!(checksum(asset));
    let signed = sign_max_approve(payer, asset);
    payload["extensions"] = serde_json::json!({
        "erc20ApprovalGasSponsoring": {
            "info": {
                "from": checksum(payer.address()),
                "asset": checksum(asset),
                "spender": checksum(PERMIT2_ADDRESS),
                "amount": U256::MAX.to_string(),
                "signedTransaction": signed,
                "version": "1"
            }
        }
    });
    (payload, serde_json::to_value(&reqs).expect("reqs"))
}

async fn prepare_anvil_erc20() -> Option<(AnvilGuard, String, Address, Address)> {
    let (guard, url) = spawn_anvil()?;
    if !wait_anvil(&url).await {
        return None;
    }
    let rpc = url.to_string();
    let provider = provider_wallet_at(&rpc, ANVIL_KEY);
    let inner = r402_evm::chain::Eip155MetaTransactionProvider::inner(&provider);
    let raw = include_str!("mock_erc20.hex");
    let trimmed = raw.trim();
    let hex_body = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    let bytecode = hex::decode(hex_body).expect("bytecode");
    anvil_set_code(inner, TOKEN, bytecode.into()).await;
    anvil_set_code(
        inner,
        X402_UPTO_PERMIT2_PROXY,
        ALWAYS_SUCCEED.to_vec().into(),
    )
    .await;
    let payer = payer_wallet().address();
    mint_tokens(&provider, TOKEN, payer, U256::from(1_000_000_000u64)).await;
    anvil_set_balance(inner, payer, U256::ZERO).await;
    Some((guard, rpc, payer, TOKEN))
}

#[tokio::test]
async fn anvil_erc20_verify_and_settle_funds_approves_and_settles() {
    let Some((_guard, rpc, payer, token)) = prepare_anvil_erc20().await else {
        return;
    };
    let provider = provider_wallet_at(&rpc, ANVIL_KEY);
    let inner = r402_evm::chain::Eip155MetaTransactionProvider::inner(&provider);
    let fac = Eip155UptoFacilitator::try_new(provider_wallet_at(&rpc, ANVIL_KEY)).expect("try_new");
    let payer_signer = payer_wallet();
    let (payload, requirements) =
        signed_upto_erc20_for(&payer_signer, token, facilitator_addr()).await;
    let nonce_before = inner.get_transaction_count(payer).await.expect("nonce");
    let verify = fac
        .verify(VerifyRequest::from(serde_json::json!({
            "x402Version": 2,
            "paymentPayload": payload,
            "paymentRequirements": requirements,
        })))
        .await
        .expect("verify");
    assert!(matches!(verify, VerifyResponse::Valid { .. }));
    let settle = fac
        .settle(SettleRequest::from(serde_json::json!({
            "x402Version": 2,
            "paymentPayload": payload,
            "paymentRequirements": requirements,
        })))
        .await
        .expect("settle");
    assert!(matches!(settle, SettleResponse::Success { .. }));
    assert_eq!(
        token_allowance(inner, token, payer).await,
        U256::MAX,
        "approve must land at maxUint256"
    );
    let nonce_after = inner.get_transaction_count(payer).await.expect("nonce");
    assert_eq!(
        nonce_after,
        nonce_before + 1,
        "exactly one payer approve tx"
    );
}

#[tokio::test]
async fn anvil_erc20_retry_skips_second_approve() {
    let Some((_guard, rpc, payer, token)) = prepare_anvil_erc20().await else {
        return;
    };
    let provider = provider_wallet_at(&rpc, ANVIL_KEY);
    let inner = r402_evm::chain::Eip155MetaTransactionProvider::inner(&provider);
    let fac = Eip155UptoFacilitator::try_new(provider_wallet_at(&rpc, ANVIL_KEY)).expect("try_new");
    let payer_signer = payer_wallet();
    let (payload, requirements) =
        signed_upto_erc20_for(&payer_signer, token, facilitator_addr()).await;
    fac.verify(VerifyRequest::from(serde_json::json!({
        "x402Version": 2,
        "paymentPayload": payload,
        "paymentRequirements": requirements,
    })))
    .await
    .expect("verify");
    anvil_set_code(
        inner,
        X402_UPTO_PERMIT2_PROXY,
        ALWAYS_REVERT.to_vec().into(),
    )
    .await;
    let first = fac
        .settle(SettleRequest::from(serde_json::json!({
            "x402Version": 2,
            "paymentPayload": payload,
            "paymentRequirements": requirements,
        })))
        .await;
    assert!(first.is_err(), "proxy revert must fail settle: {first:?}");
    let nonce_after_fail = inner.get_transaction_count(payer).await.expect("nonce");
    anvil_set_code(
        inner,
        X402_UPTO_PERMIT2_PROXY,
        ALWAYS_SUCCEED.to_vec().into(),
    )
    .await;
    let second = fac
        .settle(SettleRequest::from(serde_json::json!({
            "x402Version": 2,
            "paymentPayload": payload,
            "paymentRequirements": requirements,
        })))
        .await
        .expect("retry settle");
    assert!(matches!(second, SettleResponse::Success { .. }));
    let nonce_after_ok = inner.get_transaction_count(payer).await.expect("nonce");
    assert_eq!(
        nonce_after_ok, nonce_after_fail,
        "retry must not send a second approve"
    );
}

#[tokio::test]
async fn anvil_erc20_parallel_settles_share_one_approve() {
    let Some((_guard, rpc, payer, token)) = prepare_anvil_erc20().await else {
        return;
    };
    let provider = provider_wallet_at(&rpc, ANVIL_KEY);
    let inner = r402_evm::chain::Eip155MetaTransactionProvider::inner(&provider);
    let fac = Arc::new(
        Eip155UptoFacilitator::try_new(provider_wallet_at(&rpc, ANVIL_KEY)).expect("try_new"),
    );
    let payer_signer = payer_wallet();
    let (p1, r1) = signed_upto_erc20_for(&payer_signer, token, facilitator_addr()).await;
    let (p2, r2) = signed_upto_erc20_for(&payer_signer, token, facilitator_addr()).await;
    let nonce_before = inner.get_transaction_count(payer).await.expect("nonce");
    let fac_a = Arc::clone(&fac);
    let fac_b = Arc::clone(&fac);
    let a = tokio::spawn(async move {
        fac_a
            .settle(SettleRequest::from(serde_json::json!({
                "x402Version": 2,
                "paymentPayload": p1,
                "paymentRequirements": r1,
            })))
            .await
    });
    let b = tokio::spawn(async move {
        fac_b
            .settle(SettleRequest::from(serde_json::json!({
                "x402Version": 2,
                "paymentPayload": p2,
                "paymentRequirements": r2,
            })))
            .await
    });
    let ra = a.await.expect("join a").expect("settle a");
    let rb = b.await.expect("join b").expect("settle b");
    assert!(matches!(ra, SettleResponse::Success { .. }));
    assert!(matches!(rb, SettleResponse::Success { .. }));
    let nonce_after = inner.get_transaction_count(payer).await.expect("nonce");
    assert_eq!(nonce_after, nonce_before + 1, "one approve for two settles");
}
