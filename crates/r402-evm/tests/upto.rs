//! In-process upto-scheme tests. No live RPC. No HTTP E2E.

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
use std::sync::Arc;

use alloy_network::EthereumWallet;
use alloy_primitives::{Address, U256, address, hex};
use alloy_signer_local::PrivateKeySigner;
use r402_client::SchemeClient;
use r402_evm::chain::{ChecksummedAddress, Eip155ChainProvider, Eip155ChainReference};
use r402_evm::upto::payload::UptoPaymentRequirementsExtra;
use r402_evm::{
    Eip155Upto, Eip155UptoClient, Eip155UptoFacilitator, PERMIT2_ADDRESS, USDC,
    X402_UPTO_PERMIT2_PROXY,
};
use r402_facilitator::Facilitator;
use r402_protocol::error::{AsPaymentProblem, ErrorReason, FacilitatorError, VerificationError};
use r402_protocol::payment::{
    PaymentRequired, ResourceInfo, SettleRequest, VerifyRequest, VerifyResponse,
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

#[derive(Clone)]
struct RpcScript {
    asset: Address,
    payer: Address,
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
                    "0x6080604052"
                } else if addr == self.payer {
                    "0x"
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
            asset,
            payer,
            allowance: None,
            balance: U256::from(1_000_000_u64),
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
            asset,
            payer,
            allowance: None,
            balance: U256::from(1_000_000_u64),
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
    let asset = USDC::base().address;
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
    payload["extensions"] = serde_json::json!({
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
    });
    mount_rpc(
        &server,
        RpcScript {
            asset,
            payer,
            allowance: None,
            balance: U256::from(1_000_000_u64),
        },
    )
    .await;
    let fac = Eip155UptoFacilitator::try_new(provider_at(&server.uri()))
        .expect("try_new")
        .with_erc20_approval_gas_sponsoring();
    let resp = fac
        .verify(VerifyRequest::from(serde_json::json!({
            "x402Version": 2,
            "paymentPayload": payload,
            "paymentRequirements": tag.requirements,
        })))
        .await
        .expect("erc20 catch continues past 412");
    assert!(matches!(resp, VerifyResponse::Valid { .. }), "got {resp:?}");
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
