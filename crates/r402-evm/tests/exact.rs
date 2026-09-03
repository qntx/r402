//! In-process exact-scheme tests. No live RPC. No HTTP E2E.
//!
//! Live RPC and HTTP×EVM Sequential E2E live in sibling integration tests:
//!
//! - `R402_RPC_URLS` — comma-separated `chain_id=url` pairs for
//!   `tests/onchain_deployment.rs`. Unset skips those tests (no `#[ignore]`).
//! - `R402_ANVIL=1` — opt into Anvil at `http://127.0.0.1:8545` for
//!   on-chain EIP-3009 verify/settle and `r402-http` `e2e_exact_evm`.

#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::doc_markdown,
    clippy::excessive_nesting,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used,
    reason = "idiomatic test-code patterns"
)]

use std::str::FromStr;
use std::sync::Arc;

use alloy_network::EthereumWallet;
use alloy_primitives::{Address, Bytes, U256, address, hex};
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{SolValue, sol};

sol! {
    struct TestSig6492 {
        address factory;
        bytes factoryCalldata;
        bytes innerSig;
    }
}
use r402_client::SchemeClient;
use r402_evm::chain::{Eip155ChainProvider, Eip155ChainReference};
use r402_evm::exact::payload::{ExactPayload, PaymentRequirementsExtra};
use r402_evm::{
    AssetTransferMethod, Eip155Exact, Eip155ExactClient, Eip155ExactFacilitator, PERMIT2_ADDRESS,
    USDC,
};
use r402_facilitator::Facilitator;
use r402_protocol::error::{AsPaymentProblem, FacilitatorError, VerificationError};
use r402_protocol::payment::{
    PaymentRequired, ResourceInfo, SettleRequest, SettleResponse, VerifyRequest,
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

#[test]
fn price_tag_is_three_arg_eip3009_default() {
    let tag = Eip155Exact::price_tag(pay_to(), USDC::base().amount(1_000_000u64), None);
    assert_eq!(tag.requirements.scheme, "exact");
    assert_eq!(tag.requirements.network.to_string(), "eip155:8453");
    assert_eq!(tag.requirements.amount, "1000000");
    assert_eq!(
        tag.requirements.pay_to,
        "0x1111111111111111111111111111111111111111"
    );
    let extra: PaymentRequirementsExtra =
        serde_json::from_value(tag.requirements.extra.unwrap()).unwrap();
    assert_eq!(extra.name, "USD Coin");
    assert_eq!(extra.version, "2");
    assert!(extra.asset_transfer_method.is_none());
}

#[test]
fn price_tag_third_arg_selects_permit2() {
    let tag = Eip155Exact::price_tag(
        pay_to(),
        USDC::base().amount(1_000_000u64),
        Some(AssetTransferMethod::Permit2),
    );
    let extra: PaymentRequirementsExtra =
        serde_json::from_value(tag.requirements.extra.unwrap()).unwrap();
    assert_eq!(
        extra.asset_transfer_method,
        Some(AssetTransferMethod::Permit2)
    );
}

#[test]
fn exact_scheme_id_and_payment_flows() {
    let scheme = Eip155Exact;
    assert_eq!(scheme.namespace(), "eip155");
    assert_eq!(SchemeNetworkServer::scheme(&scheme), "exact");
    assert_eq!(scheme.default_asset_transfer_method(), "eip3009");
    let expected = r402_server::PaymentFlowConfig::authorization_and_upfront();
    assert_eq!(scheme.payment_flows().get("eip3009"), Some(&expected));
    assert_eq!(scheme.payment_flows().get("permit2"), Some(&expected));
    assert_eq!(expected.default, PaymentFlowName::Authorization);
}

fn try_new_question_mark() -> Result<(), FacilitatorError> {
    let _fac = Eip155ExactFacilitator::try_new(dummy_provider())?;
    Ok(())
}

#[test]
fn try_new_question_mark_compiles() {
    try_new_question_mark().expect("try_new is currently infallible");
}

#[test]
fn polygon_and_arbitrum_sepolia_eip712_name_is_usd_coin() {
    let polygon = USDC::polygon().eip712.as_ref().expect("polygon eip712");
    assert_eq!(polygon.name, "USD Coin");
    assert_eq!(polygon.version, "2");
    let arb = USDC::arbitrum_sepolia()
        .eip712
        .as_ref()
        .expect("arb sepolia eip712");
    assert_eq!(arb.name, "USD Coin");
    assert_eq!(arb.version, "2");
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

const EIP6492_MAGIC: [u8; 32] =
    hex!("6492649264926492649264926492649264926492649264926492649264926492");

fn wrap_6492(factory: Address, inner: Bytes) -> Bytes {
    let encoded = TestSig6492 {
        factory,
        factoryCalldata: Bytes::from(vec![0xde, 0xad]),
        innerSig: inner,
    }
    .abi_encode_params();
    let mut out = encoded;
    out.extend_from_slice(&EIP6492_MAGIC);
    Bytes::from(out)
}

fn checksum(addr: Address) -> String {
    addr.to_checksum(None)
}

fn eip3009_request(payer: Address, asset: Address, signature: &Bytes) -> serde_json::Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let pay_to = pay_to();
    serde_json::json!({
        "x402Version": 2,
        "paymentPayload": {
            "x402Version": 2,
            "accepted": {
                "scheme": "exact",
                "network": "eip155:8453",
                "amount": "1000000",
                "payTo": checksum(pay_to),
                "maxTimeoutSeconds": 300,
                "asset": checksum(asset),
                "extra": { "name": "USD Coin", "version": "2" }
            },
            "payload": {
                "signature": signature,
                "authorization": {
                    "from": checksum(payer),
                    "to": checksum(pay_to),
                    "value": "1000000",
                    "validAfter": "0",
                    "validBefore": (now + 3600).to_string(),
                    "nonce": "0x1111111111111111111111111111111111111111111111111111111111111111"
                }
            }
        },
        "paymentRequirements": {
            "scheme": "exact",
            "network": "eip155:8453",
            "amount": "1000000",
            "payTo": checksum(pay_to),
            "maxTimeoutSeconds": 300,
            "asset": checksum(asset),
            "extra": { "name": "USD Coin", "version": "2" }
        }
    })
}

fn permit2_request(
    payer: Address,
    asset: Address,
    extensions: &serde_json::Value,
) -> serde_json::Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let pay_to = pay_to();
    let proxy = r402_evm::exact::X402_EXACT_PERMIT2_PROXY;
    serde_json::json!({
        "x402Version": 2,
        "paymentPayload": {
            "x402Version": 2,
            "accepted": {
                "scheme": "exact",
                "network": "eip155:8453",
                "amount": "1000000",
                "payTo": checksum(pay_to),
                "maxTimeoutSeconds": 300,
                "asset": checksum(asset),
                "extra": { "name": "USD Coin", "version": "2", "assetTransferMethod": "permit2" }
            },
            "payload": {
                "signature": "0x1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111",
                "permit2Authorization": {
                    "from": checksum(payer),
                    "permitted": { "token": checksum(asset), "amount": "1000000" },
                    "spender": checksum(proxy),
                    "nonce": "1",
                    "deadline": (now + 3600).to_string(),
                    "witness": { "to": checksum(pay_to), "validAfter": "0" }
                }
            },
            "extensions": extensions
        },
        "paymentRequirements": {
            "scheme": "exact",
            "network": "eip155:8453",
            "amount": "1000000",
            "payTo": checksum(pay_to),
            "maxTimeoutSeconds": 300,
            "asset": checksum(asset),
            "extra": { "name": "USD Coin", "version": "2", "assetTransferMethod": "permit2" }
        }
    })
}

#[derive(Clone)]
struct RpcScript {
    asset: Address,
    payer: Address,
    asset_deployed: bool,
    payer_code: Vec<u8>,
    allowance: Option<U256>,
    balance: U256,
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
        match method {
            "eth_getCode" => {
                let addr = body
                    .get("params")
                    .and_then(|p| p.get(0))
                    .and_then(serde_json::Value::as_str)
                    .and_then(|s| s.parse::<Address>().ok())
                    .unwrap_or(Address::ZERO);
                let code = if addr == self.asset {
                    if self.asset_deployed {
                        "0x6080604052"
                    } else {
                        "0x"
                    }
                } else if addr == self.payer {
                    if self.payer_code.is_empty() {
                        "0x"
                    } else {
                        return ok(serde_json::json!(format!(
                            "0x{}",
                            hex::encode(&self.payer_code)
                        )));
                    }
                } else {
                    "0x"
                };
                ok(serde_json::json!(code))
            }
            "eth_call" => {
                let call = body.get("params").and_then(|p| p.get(0));
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
                ok(serde_json::json!(
                    "0x0000000000000000000000000000000000000000000000000000000000000000"
                ))
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

fn verify_reason(err: FacilitatorError) -> String {
    match err {
        FacilitatorError::Verification(e) => e.as_payment_problem().reason().as_str().to_owned(),
        other => other.to_string(),
    }
}

#[tokio::test]
async fn try_new_supported_is_exact_on_provider_chain() {
    let fac = Eip155ExactFacilitator::try_new(dummy_provider()).expect("try_new");
    let supported = fac.supported().await.expect("supported");
    let kind = supported.kinds.first().expect("one kind");
    assert_eq!(kind.scheme, "exact");
    assert_eq!(kind.network, "eip155:8453");
    assert_eq!(kind.x402_version, 2);
    assert!(
        !supported.signers.is_empty(),
        "try_new provider must advertise signers"
    );
}

#[tokio::test]
async fn verify_malformed_payload_is_invalid_format() {
    let fac = Eip155ExactFacilitator::try_new(dummy_provider()).expect("try_new");
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
async fn client_accepts_and_signs_eip3009_locally() {
    let (signer, _) = wallet();
    let client = Eip155ExactClient::new(Arc::new(signer));
    let tag = Eip155Exact::price_tag(pay_to(), USDC::base().amount(1_000_000u64), None);
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()]);
    let candidates = client.accept(&required);
    assert_eq!(candidates.len(), 1, "one eip3009 accept");
    let b64 = candidates
        .first()
        .expect("one eip3009 accept")
        .sign()
        .await
        .expect("local EIP-712 sign");
    let json = r402_protocol::payment::Base64Bytes(b64.into_bytes())
        .decode()
        .expect("b64");
    let payload: r402_evm::exact::payload::v2::PaymentPayload =
        serde_json::from_slice(&json).expect("payload");
    assert!(matches!(payload.payload, ExactPayload::Eip3009(_)));
    assert_eq!(payload.accepted.network.to_string(), "eip155:8453");
}

#[tokio::test]
async fn client_accepts_and_signs_permit2_locally() {
    let (signer, _) = wallet();
    let client = Eip155ExactClient::new(Arc::new(signer));
    let tag = Eip155Exact::price_tag(
        pay_to(),
        USDC::base().amount(1_000_000u64),
        Some(AssetTransferMethod::Permit2),
    );
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()]);
    let candidates = client.accept(&required);
    assert_eq!(candidates.len(), 1, "one permit2 accept");
    let b64 = candidates
        .first()
        .expect("one permit2 accept")
        .sign()
        .await
        .expect("local Permit2 sign");
    let json = r402_protocol::payment::Base64Bytes(b64.into_bytes())
        .decode()
        .expect("b64");
    let payload: r402_evm::exact::payload::v2::PaymentPayload =
        serde_json::from_slice(&json).expect("payload");
    assert!(matches!(payload.payload, ExactPayload::Permit2(_)));
    assert!(
        payload.extensions.is_empty(),
        "undeclared eip2612/erc20 must not be attached"
    );
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
    let client = Eip155ExactClient::builder(signer)
        .approver(LowAllowance)
        .auto_approve(false)
        .build();
    let tag = Eip155Exact::price_tag(
        pay_to(),
        USDC::base().amount(1_000_000u64),
        Some(AssetTransferMethod::Permit2),
    );
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
    let payload: r402_evm::exact::payload::v2::PaymentPayload =
        serde_json::from_slice(&json).expect("payload");
    assert!(payload.extensions.get(EIP2612_GAS_SPONSORING_KEY).is_some());
}

#[tokio::test]
async fn client_attaches_erc20_when_2612_absent_and_erc20_advertised() {
    use std::future::Future;
    use std::pin::Pin;

    use r402_evm::Permit2Approver;
    use r402_evm::erc20_approval::ERC20_APPROVAL_GAS_SPONSORING_KEY;
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
            Box::pin(async { Ok(U256::ZERO) })
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
    let client = Eip155ExactClient::builder(signer)
        .approver(LowAllowance)
        .auto_approve(false)
        .build();
    let tag = Eip155Exact::price_tag(
        pay_to(),
        USDC::base().amount(1_000_000u64),
        Some(AssetTransferMethod::Permit2),
    );
    let mut advertised = Extensions::new();
    advertised.insert(
        ERC20_APPROVAL_GAS_SPONSORING_KEY,
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
    let payload: r402_evm::exact::payload::v2::PaymentPayload =
        serde_json::from_slice(&json).expect("payload");
    assert!(
        payload
            .extensions
            .get(ERC20_APPROVAL_GAS_SPONSORING_KEY)
            .is_some()
    );
}

#[test]
fn find_default_asset_base_usdc() {
    let network = "eip155:8453".parse().unwrap();
    let info = r402_evm::find_default_evm_asset_info(
        "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
        &network,
    )
    .expect("base usdc");
    assert_eq!(info.symbol, "USDC");
    assert_eq!(info.decimals, 6);
}

#[tokio::test]
async fn verify_6492_empty_allowlist_is_factory_not_allowed() {
    let server = wiremock::MockServer::start().await;
    let payer = Address::repeat_byte(0xCC);
    let asset = USDC::base().address;
    let factory = Address::repeat_byte(0xF1);
    mount_rpc(
        &server,
        RpcScript {
            asset,
            payer,
            asset_deployed: true,
            payer_code: vec![],
            allowance: Some(U256::from(1_000_000_u64)),
            balance: U256::from(1_000_000_u64),
        },
    )
    .await;
    let fac = Eip155ExactFacilitator::try_new(provider_at(&server.uri())).expect("try_new");
    let sig = wrap_6492(factory, Bytes::from(vec![0u8; 65]));
    let err = fac
        .verify(VerifyRequest::from(eip3009_request(payer, asset, &sig)))
        .await
        .expect_err("empty allowlist denies 6492");
    assert_eq!(verify_reason(err), "eip6492_factory_not_allowed");
}

#[tokio::test]
async fn settle_6492_empty_allowlist_is_failure_with_empty_tx() {
    let server = wiremock::MockServer::start().await;
    let payer = Address::repeat_byte(0xCC);
    let asset = USDC::base().address;
    let factory = Address::repeat_byte(0xF1);
    mount_rpc(
        &server,
        RpcScript {
            asset,
            payer,
            asset_deployed: true,
            payer_code: vec![],
            allowance: Some(U256::from(1_000_000_u64)),
            balance: U256::from(1_000_000_u64),
        },
    )
    .await;
    let fac = Eip155ExactFacilitator::try_new(provider_at(&server.uri())).expect("try_new");
    let sig = wrap_6492(factory, Bytes::from(vec![0u8; 65]));
    let resp = fac
        .settle(SettleRequest::from(eip3009_request(payer, asset, &sig)))
        .await
        .expect("settle returns structured failure");
    match resp {
        SettleResponse::Failure {
            reason,
            transaction,
            ..
        } => {
            assert_eq!(reason.as_str(), "eip6492_factory_not_allowed");
            assert!(transaction.is_empty());
        }
        other => panic!("expected Failure, got {other:?}"),
    }
}

#[tokio::test]
async fn verify_6492_allowlisted_factory_is_not_factory_not_allowed() {
    let server = wiremock::MockServer::start().await;
    let payer = Address::repeat_byte(0xCC);
    let asset = USDC::base().address;
    let factory = Address::repeat_byte(0xF1);
    mount_rpc(
        &server,
        RpcScript {
            asset,
            payer,
            asset_deployed: true,
            payer_code: vec![],
            allowance: Some(U256::from(1_000_000_u64)),
            balance: U256::from(1_000_000_u64),
        },
    )
    .await;
    let fac = Eip155ExactFacilitator::try_new(provider_at(&server.uri()))
        .expect("try_new")
        .with_eip6492_allowed_factories(vec![factory]);
    let sig = wrap_6492(factory, Bytes::from(vec![0u8; 65]));
    let err = fac
        .verify(VerifyRequest::from(eip3009_request(payer, asset, &sig)))
        .await
        .expect_err("simulation still fails after allowlist");
    assert_ne!(verify_reason(err), "eip6492_factory_not_allowed");
}

#[tokio::test]
async fn settle_6492_allowlisted_factory_is_not_factory_not_allowed() {
    let server = wiremock::MockServer::start().await;
    let payer = Address::repeat_byte(0xCC);
    let asset = USDC::base().address;
    let factory = Address::repeat_byte(0xF1);
    mount_rpc(
        &server,
        RpcScript {
            asset,
            payer,
            asset_deployed: true,
            payer_code: vec![],
            allowance: Some(U256::from(1_000_000_u64)),
            balance: U256::from(1_000_000_u64),
        },
    )
    .await;
    let fac = Eip155ExactFacilitator::try_new(provider_at(&server.uri()))
        .expect("try_new")
        .with_eip6492_allowed_factories(vec![factory]);
    let sig = wrap_6492(factory, Bytes::from(vec![0u8; 65]));
    let outcome = fac
        .settle(SettleRequest::from(eip3009_request(payer, asset, &sig)))
        .await;
    match outcome {
        Ok(SettleResponse::Failure { reason, .. }) => {
            assert_ne!(reason.as_str(), "eip6492_factory_not_allowed");
        }
        Err(err) => assert_ne!(verify_reason(err), "eip6492_factory_not_allowed"),
        Ok(other) => panic!("unexpected success {other:?}"),
    }
}

#[tokio::test]
async fn undeployed_asset_is_asset_not_deployed_contract() {
    let server = wiremock::MockServer::start().await;
    let payer = Address::repeat_byte(0xCC);
    let asset = USDC::base().address;
    mount_rpc(
        &server,
        RpcScript {
            asset,
            payer,
            asset_deployed: false,
            payer_code: vec![],
            allowance: Some(U256::from(1_000_000_u64)),
            balance: U256::from(1_000_000_u64),
        },
    )
    .await;
    let fac = Eip155ExactFacilitator::try_new(provider_at(&server.uri())).expect("try_new");
    let sig = Bytes::from(vec![0u8; 65]);
    let err = fac
        .verify(VerifyRequest::from(eip3009_request(payer, asset, &sig)))
        .await
        .expect_err("undeployed asset");
    assert_eq!(verify_reason(err), "asset_not_deployed_contract");
}

#[tokio::test]
async fn permit2_allowance_rpc_err_without_extensions_is_412() {
    let server = wiremock::MockServer::start().await;
    let payer = Address::repeat_byte(0xCC);
    let asset = USDC::base().address;
    mount_rpc(
        &server,
        RpcScript {
            asset,
            payer,
            asset_deployed: true,
            payer_code: vec![],
            allowance: None,
            balance: U256::from(1_000_000_u64),
        },
    )
    .await;
    let fac = Eip155ExactFacilitator::try_new(provider_at(&server.uri())).expect("try_new");
    let err = fac
        .verify(VerifyRequest::from(permit2_request(
            payer,
            asset,
            &serde_json::json!({}),
        )))
        .await
        .expect_err("allowance rpc fail-closed");
    assert!(matches!(
        err,
        FacilitatorError::Verification(VerificationError::Permit2AllowanceRequired)
    ));
}

#[tokio::test]
async fn permit2_allowance_rpc_err_with_eip2612_continues() {
    let server = wiremock::MockServer::start().await;
    let payer = Address::repeat_byte(0xCC);
    let asset = USDC::base().address;
    mount_rpc(
        &server,
        RpcScript {
            asset,
            payer,
            asset_deployed: true,
            payer_code: vec![],
            allowance: None,
            balance: U256::from(1_000_000_u64),
        },
    )
    .await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let fac = Eip155ExactFacilitator::try_new(provider_at(&server.uri())).expect("try_new");
    let err = fac
        .verify(VerifyRequest::from(permit2_request(
            payer,
            asset,
            &serde_json::json!({
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
            }),
        )))
        .await
        .expect_err("simulation after eip2612 catch");
    assert!(
        !matches!(
            err,
            FacilitatorError::Verification(VerificationError::Permit2AllowanceRequired)
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn permit2_erc20_payload_stub_does_not_cover_without_facilitator_extension() {
    let server = wiremock::MockServer::start().await;
    let payer = Address::repeat_byte(0xCC);
    let asset = USDC::base().address;
    mount_rpc(
        &server,
        RpcScript {
            asset,
            payer,
            asset_deployed: true,
            payer_code: vec![],
            allowance: None,
            balance: U256::from(1_000_000_u64),
        },
    )
    .await;
    let fac = Eip155ExactFacilitator::try_new(provider_at(&server.uri())).expect("try_new");
    let err = fac
        .verify(VerifyRequest::from(permit2_request(
            payer,
            asset,
            &serde_json::json!({
                "erc20ApprovalGasSponsoring": {
                    "info": {
                        "from": checksum(payer),
                        "asset": checksum(asset),
                        "spender": checksum(PERMIT2_ADDRESS),
                        "amount": "1000000",
                        "signedTransaction": "0xdead",
                        "version": "1"
                    }
                }
            }),
        )))
        .await
        .expect_err("payload-only erc20 does not cover");
    assert!(matches!(
        err,
        FacilitatorError::Verification(VerificationError::Permit2AllowanceRequired)
    ));
}

#[tokio::test]
async fn permit2_allowance_rpc_err_with_facilitator_erc20_and_valid_tx_continues() {
    use alloy_consensus::{SignableTransaction, TxEip1559, TxEnvelope};
    use alloy_eips::eip2718::Encodable2718;
    use alloy_eips::eip2930::AccessList;
    use alloy_network::TxSignerSync;
    use alloy_primitives::TxKind;

    let server = wiremock::MockServer::start().await;
    let (signer, _) = wallet();
    let payer = signer.address();
    let asset = USDC::base().address;
    mount_rpc(
        &server,
        RpcScript {
            asset,
            payer,
            asset_deployed: true,
            payer_code: vec![],
            allowance: None,
            balance: U256::from(1_000_000_u64),
        },
    )
    .await;
    let mut calldata = vec![0x09, 0x5e, 0xa7, 0xb3];
    calldata.extend_from_slice(&[0u8; 12]);
    calldata.extend_from_slice(PERMIT2_ADDRESS.as_slice());
    calldata.extend_from_slice(&[0u8; 32]);
    let mut tx = TxEip1559 {
        chain_id: 8453,
        nonce: 0,
        gas_limit: 80_000,
        max_fee_per_gas: 1_000_000_000,
        max_priority_fee_per_gas: 1_000_000,
        to: TxKind::Call(asset),
        value: U256::ZERO,
        access_list: AccessList::default(),
        input: calldata.into(),
    };
    let sig = signer.sign_transaction_sync(&mut tx).expect("sign");
    let signed = format!(
        "0x{}",
        hex::encode(TxEnvelope::Eip1559(tx.into_signed(sig)).encoded_2718())
    );
    let fac = Eip155ExactFacilitator::try_new(provider_at(&server.uri()))
        .expect("try_new")
        .with_erc20_approval_gas_sponsoring();
    let err = fac
        .verify(VerifyRequest::from(permit2_request(
            payer,
            asset,
            &serde_json::json!({
                "erc20ApprovalGasSponsoring": {
                    "info": {
                        "from": checksum(payer),
                        "asset": checksum(asset),
                        "spender": checksum(PERMIT2_ADDRESS),
                        "amount": "1000000",
                        "signedTransaction": signed,
                        "version": "1"
                    }
                }
            }),
        )))
        .await
        .expect_err("simulation after erc20 catch");
    assert!(
        !matches!(
            err,
            FacilitatorError::Verification(VerificationError::Permit2AllowanceRequired)
        ),
        "got {err:?}"
    );
}
