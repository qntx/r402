//! In-process batch-settlement tests. No live RPC. No HTTP E2E.

#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::clone_on_ref_ptr,
    clippy::doc_markdown,
    clippy::excessive_nesting,
    clippy::expect_used,
    clippy::get_first,
    clippy::indexing_slicing,
    clippy::match_same_arms,
    clippy::missing_assert_message,
    clippy::option_if_let_else,
    clippy::panic,
    clippy::significant_drop_in_scrutinee,
    clippy::significant_drop_tightening,
    clippy::unwrap_used,
    reason = "idiomatic test-code patterns"
)]

use std::future::Future;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use alloy_network::EthereumWallet;
use alloy_primitives::{Address, B256, Bytes, TxHash, U256, address, hex, keccak256};
use alloy_rpc_types_eth::TransactionReceipt;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{SolStruct, SolValue, eip712_domain, sol};

sol! {
    struct TestSig6492 {
        address factory;
        bytes factoryCalldata;
        bytes innerSig;
    }

    struct ReceiveWithAuthorization {
        address from;
        address to;
        uint256 value;
        uint256 validAfter;
        uint256 validBefore;
        bytes32 nonce;
    }

}
mod permit2_td {
    use alloy_sol_types::sol;
    sol! {
        struct TokenPermissions {
            address token;
            uint256 amount;
        }
        struct DepositWitness {
            bytes32 channelId;
        }
        struct PermitWitnessTransferFrom {
            TokenPermissions permitted;
            address spender;
            uint256 nonce;
            uint256 deadline;
            DepositWitness witness;
        }
    }
}

use r402_client::SchemeClient;
use r402_evm::batch_settlement::channel::compute_channel_id;
use r402_evm::batch_settlement::errors::{
    CHANNEL_ID_MISMATCH, CHANNEL_NOT_FOUND, EIP6492_FACTORY_NOT_ALLOWED, INVALID_PAYLOAD_TYPE,
    INVALID_RECEIVE_AUTHORIZATION_SIGNATURE, PERMIT2_ALLOWANCE_REQUIRED,
};
use r402_evm::batch_settlement::payload::{
    BatchSettlementExtra, BatchSettlementPayload, ChannelConfig, ClaimVoucherBody,
    DepositAuthorization, DepositBody, DepositErc3009Authorization, DepositPermit2Authorization,
    DepositTokenPermissions, DepositWitness, PERMIT2_DEPOSIT_COLLECTOR_ADDRESS, VoucherClaim,
    VoucherFields,
};
use r402_evm::batch_settlement::voucher::sign_voucher;
use r402_evm::chain::{
    ChecksummedAddress, Eip155ChainProvider, Eip155ChainReference, Eip155MetaTransactionProvider,
    InnerProvider, MetaTransaction, MetaTransactionSendError, TokenAmount,
};
use r402_evm::signer::SignerLike;
use r402_evm::{
    Eip155BatchSettlement, Eip155BatchSettlementClient, Eip155BatchSettlementFacilitator,
    PERMIT2_ADDRESS, USDC,
};
use r402_facilitator::{Facilitator, InMemoryPendingSettlementStore, PendingSettlementStore};
use r402_protocol::error::{AsPaymentProblem, FacilitatorError, VerificationError};
use r402_protocol::network::ChainProvider;
use r402_protocol::payment::{
    PaymentRequired, ResourceInfo, SettleRequest, SettleResponse, VerifyRequest, VerifyResponse,
};
use r402_protocol::scheme::{BatchSettlementScheme, SchemeId};
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

fn sample_extra() -> BatchSettlementExtra {
    BatchSettlementExtra {
        receiver_authorizer: ChecksummedAddress(Address::repeat_byte(0x33)),
        withdraw_delay: 900,
        name: "USD Coin".into(),
        version: "2".into(),
        asset_transfer_method: None,
    }
}

#[test]
fn price_tag_embeds_batch_settlement_extra() {
    let extra = sample_extra();
    let tag = Eip155BatchSettlement::price_tag(pay_to(), USDC::base().amount(50u64), extra.clone());
    assert_eq!(tag.requirements.scheme, "batch-settlement");
    assert_eq!(tag.requirements.network.to_string(), "eip155:8453");
    let decoded: BatchSettlementExtra =
        serde_json::from_value(tag.requirements.extra.unwrap()).unwrap();
    assert_eq!(decoded.withdraw_delay, 900);
    assert_eq!(decoded.receiver_authorizer, extra.receiver_authorizer);
}

#[test]
fn batch_settlement_scheme_id_and_payment_flows() {
    let scheme = Eip155BatchSettlement;
    assert_eq!(scheme.namespace(), "eip155");
    assert_eq!(SchemeNetworkServer::scheme(&scheme), "batch-settlement");
    assert_eq!(scheme.default_asset_transfer_method(), "eip3009");
    let expected = r402_server::PaymentFlowConfig::authorization_only();
    assert_eq!(scheme.payment_flows().get("eip3009"), Some(&expected));
    assert_eq!(scheme.payment_flows().get("permit2"), Some(&expected));
    assert_eq!(expected.default, PaymentFlowName::Authorization);
}

fn try_new_question_mark() -> Result<(), FacilitatorError> {
    let _fac = Eip155BatchSettlementFacilitator::try_new(dummy_provider())?;
    Ok(())
}

#[test]
fn try_new_question_mark_compiles() {
    try_new_question_mark().expect("try_new is currently infallible");
}

#[tokio::test]
async fn try_new_supported_is_batch_settlement_on_provider_chain() {
    let fac = Eip155BatchSettlementFacilitator::try_new(dummy_provider()).expect("try_new");
    let supported = fac.supported().await.expect("supported");
    let kind = supported.kinds.first().expect("one kind");
    assert_eq!(kind.scheme, "batch-settlement");
    assert_eq!(kind.network, "eip155:8453");
    assert_eq!(kind.x402_version, 2);
    assert!(
        !supported.signers.is_empty(),
        "try_new provider must advertise signers"
    );
}

#[tokio::test]
async fn verify_malformed_payload_is_invalid_format() {
    let fac = Eip155BatchSettlementFacilitator::try_new(dummy_provider()).expect("try_new");
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
async fn voucher_verify_stays_offchain_settle_is_invalid_payload_type() {
    let (signer, _) = wallet();
    let client = Eip155BatchSettlementClient::new(Arc::new(signer));
    let tag =
        Eip155BatchSettlement::price_tag(pay_to(), USDC::base().amount(50u64), sample_extra());
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()]);
    let candidates = client.accept(&required);
    assert_eq!(candidates.len(), 1, "one batch-settlement accept");
    let b64 = candidates
        .first()
        .expect("one accept")
        .sign()
        .await
        .expect("local voucher sign");
    let json = r402_protocol::payment::Base64Bytes(b64.into_bytes())
        .decode()
        .expect("b64");
    let payload: r402_evm::batch_settlement::payload::v2::PaymentPayload =
        serde_json::from_slice(&json).expect("payload");
    assert!(matches!(
        payload.payload,
        BatchSettlementPayload::Voucher { .. }
    ));
    let channel_id = payload.payload.voucher().expect("voucher").channel_id;

    let requirements: r402_evm::batch_settlement::payload::v2::PaymentRequirements =
        serde_json::from_value(serde_json::to_value(&tag.requirements).unwrap()).unwrap();
    let typed = r402_evm::batch_settlement::payload::v2::VerifyRequest {
        x402_version: r402_protocol::payment::V2,
        payment_payload: payload,
        payment_requirements: requirements,
    };
    let request = VerifyRequest::try_from(typed).expect("typed verify");
    let fac = Eip155BatchSettlementFacilitator::try_new(dummy_provider()).expect("try_new");
    let response = fac.verify(request.clone()).await.expect("verify");
    assert!(matches!(response, VerifyResponse::Valid { .. }));

    let settled = fac
        .settle(SettleRequest::from(request))
        .await
        .expect("settle returns structured failure");
    match settled {
        SettleResponse::Failure {
            reason,
            transaction,
            ..
        } => {
            assert_eq!(reason.as_str(), INVALID_PAYLOAD_TYPE);
            assert!(transaction.is_empty());
        }
        other => panic!("expected Failure, got {other:?}"),
    }
    assert_eq!(
        fac.store().get(&channel_id).charged_cumulative.0,
        U256::ZERO,
        "voucher settle must not charge ChannelStore"
    );
}

#[tokio::test]
async fn client_sign_rejects_withdraw_delay_uint40_overflow() {
    let (signer, _) = wallet();
    let client = Eip155BatchSettlementClient::new(Arc::new(signer));
    let mut extra = sample_extra();
    extra.withdraw_delay = u64::MAX;
    let tag = Eip155BatchSettlement::price_tag(pay_to(), USDC::base().amount(50u64), extra);
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()]);
    let err = client
        .accept(&required)
        .first()
        .expect("one accept")
        .sign()
        .await
        .expect_err("withdrawDelay >= 2^40 must not panic");
    assert!(
        matches!(err, r402_protocol::error::ClientError::Signing(_)),
        "got {err:?}"
    );
}

const FIXED_HASH: TxHash = TxHash::repeat_byte(0x11);
const EIP6492_MAGIC: [u8; 32] =
    hex!("6492649264926492649264926492649264926492649264926492649264926492");

fn sel(sig: &str) -> [u8; 4] {
    let hash = keccak256(sig.as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

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

fn receipt_json(hash: TxHash) -> serde_json::Value {
    serde_json::json!({
        "transactionHash": hash,
        "transactionIndex": "0x0",
        "blockHash": "0x2222222222222222222222222222222222222222222222222222222222222222",
        "blockNumber": "0x1",
        "from": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "to": "0x4020074e9dF2ce1deE5A9C1b5c3f541D02a10003",
        "cumulativeGasUsed": "0x1",
        "gasUsed": "0x1",
        "effectiveGasPrice": "0x1",
        "contractAddress": null,
        "logs": [],
        "logsBloom": format!("0x{}", "0".repeat(512)),
        "status": "0x1",
        "type": "0x2"
    })
}

fn success_receipt(hash: TxHash) -> TransactionReceipt {
    serde_json::from_value(receipt_json(hash)).expect("receipt json")
}

#[derive(Clone, Copy)]
enum SendMode {
    Success,
    ReceiptWait,
}

struct RecordingProvider {
    inner: Eip155ChainProvider,
    sent: Arc<Mutex<Vec<Bytes>>>,
    send_mode: Arc<Mutex<SendMode>>,
}

impl ChainProvider for RecordingProvider {
    fn signer_addresses(&self) -> Vec<String> {
        self.inner.signer_addresses()
    }

    fn chain_id(&self) -> r402_protocol::network::ChainId {
        self.inner.chain_id()
    }
}

impl Eip155MetaTransactionProvider for RecordingProvider {
    type Error = MetaTransactionSendError;
    type Inner = InnerProvider;

    fn inner(&self) -> &Self::Inner {
        Eip155MetaTransactionProvider::inner(&self.inner)
    }

    fn chain(&self) -> &Eip155ChainReference {
        Eip155MetaTransactionProvider::chain(&self.inner)
    }

    fn send_transaction(
        &self,
        tx: MetaTransaction,
    ) -> impl Future<Output = Result<TransactionReceipt, Self::Error>> + Send {
        self.sent.lock().unwrap().push(tx.calldata);
        std::future::ready(match *self.send_mode.lock().unwrap() {
            SendMode::Success => Ok(success_receipt(FIXED_HASH)),
            SendMode::ReceiptWait => Err(MetaTransactionSendError::ReceiptWait {
                hash: FIXED_HASH,
                source: alloy_provider::PendingTransactionError::FailedToRegister,
            }),
        })
    }
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

#[derive(Clone)]
struct RpcScript {
    payer_code: Arc<Mutex<Vec<u8>>>,
    mined: Arc<Mutex<bool>>,
    allowance: Arc<Mutex<Option<U256>>>,
    channel_balance: Arc<Mutex<u128>>,
}

impl Default for RpcScript {
    fn default() -> Self {
        Self {
            payer_code: Arc::new(Mutex::new(Vec::new())),
            mined: Arc::new(Mutex::new(false)),
            allowance: Arc::new(Mutex::new(Some(U256::from(1_000_000_u64)))),
            channel_balance: Arc::new(Mutex::new(1000)),
        }
    }
}

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
                let code = self.payer_code.lock().unwrap();
                if code.is_empty() {
                    ok(serde_json::json!("0x"))
                } else {
                    ok(serde_json::json!(format!("0x{}", hex::encode(&*code))))
                }
            }
            "eth_getTransactionReceipt" => {
                if *self.mined.lock().unwrap() {
                    ok(receipt_json(FIXED_HASH))
                } else {
                    ok(serde_json::Value::Null)
                }
            }
            "eth_call" => {
                let call = body.get("params").and_then(|p| p.get(0));
                let data = call
                    .and_then(|c| c.get("data").or_else(|| c.get("input")))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("0x");
                let selector = data.get(2..10).unwrap_or("").to_ascii_lowercase();
                if selector == "70a08231" {
                    return ok(serde_json::json!(format!(
                        "0x{:064x}",
                        U256::from(1_000_000_u64)
                    )));
                }
                if selector == "dd62ed3e" {
                    return match *self.allowance.lock().unwrap() {
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
                let balance = *self.channel_balance.lock().unwrap();
                ok(serde_json::json!(format!(
                    "0x{:064x}{:064x}",
                    balance, 0_u128
                )))
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

struct FacHarness {
    fac: Eip155BatchSettlementFacilitator<RecordingProvider>,
    sent: Arc<Mutex<Vec<Bytes>>>,
    send_mode: Arc<Mutex<SendMode>>,
    rpc: RpcScript,
    pending: Arc<InMemoryPendingSettlementStore>,
}

async fn harness() -> FacHarness {
    let rpc = RpcScript::default();
    let server = wiremock::MockServer::start().await;
    mount_rpc(&server, rpc.clone()).await;
    let sent = Arc::new(Mutex::new(Vec::new()));
    let send_mode = Arc::new(Mutex::new(SendMode::Success));
    let pending = Arc::new(InMemoryPendingSettlementStore::new());
    let provider = RecordingProvider {
        inner: provider_at(&server.uri()),
        sent: Arc::clone(&sent),
        send_mode: Arc::clone(&send_mode),
    };
    let pending_store: Arc<dyn PendingSettlementStore> = pending.clone();
    let fac = Eip155BatchSettlementFacilitator::try_new(provider)
        .expect("try_new")
        .with_pending_store(pending_store);
    FacHarness {
        fac,
        sent,
        send_mode,
        rpc,
        pending,
    }
}

async fn recording_fac() -> (
    Eip155BatchSettlementFacilitator<RecordingProvider>,
    Arc<Mutex<Vec<Bytes>>>,
) {
    let h = harness().await;
    (h.fac, h.sent)
}

fn sample_config(payer: Address) -> ChannelConfig {
    ChannelConfig {
        payer,
        payer_authorizer: payer,
        receiver: pay_to(),
        receiver_authorizer: Address::repeat_byte(0x33),
        token: USDC::base().address,
        withdraw_delay: 900,
        salt: B256::ZERO,
    }
}

fn requirements_for(
    config: &ChannelConfig,
) -> r402_evm::batch_settlement::payload::v2::PaymentRequirements {
    r402_evm::batch_settlement::payload::v2::PaymentRequirements::new(
        BatchSettlementScheme,
        "eip155:8453".parse().expect("chain"),
        TokenAmount::from(U256::from(50_u64)),
        ChecksummedAddress(config.receiver),
        ChecksummedAddress(config.token),
        3600,
    )
    .with_extra(sample_extra())
}

fn verify_request(
    payload: BatchSettlementPayload,
    requirements: r402_evm::batch_settlement::payload::v2::PaymentRequirements,
) -> VerifyRequest {
    let payment =
        r402_evm::batch_settlement::payload::v2::PaymentPayload::new(requirements.clone(), payload);
    let typed = r402_evm::batch_settlement::payload::v2::VerifyRequest {
        x402_version: r402_protocol::payment::V2,
        payment_payload: payment,
        payment_requirements: requirements,
    };
    VerifyRequest::try_from(typed).expect("typed")
}

fn settle_request(
    payload: BatchSettlementPayload,
    requirements: r402_evm::batch_settlement::payload::v2::PaymentRequirements,
) -> SettleRequest {
    SettleRequest::from(verify_request(payload, requirements))
}

fn verify_reason(err: FacilitatorError) -> String {
    match err {
        FacilitatorError::Verification(e) => e.as_payment_problem().reason().as_str().to_owned(),
        other => other.to_string(),
    }
}

async fn signed_deposit(
    signer: &PrivateKeySigner,
) -> (BatchSettlementPayload, ChannelConfig, Bytes) {
    let payer = signer.address();
    let config = sample_config(payer);
    let channel_id = compute_channel_id(&config, 8453).expect("id");
    let voucher = sign_voucher(signer, channel_id, U256::from(50_u64), 8453)
        .await
        .expect("voucher");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let salt = B256::repeat_byte(0xab);
    let nonce = keccak256((channel_id, U256::from_be_bytes(salt.0)).abi_encode());
    let domain = eip712_domain! {
        name: "USD Coin".to_owned(),
        version: "2".to_owned(),
        chain_id: 8453_u64,
        verifying_contract: USDC::base().address,
    };
    let typed = ReceiveWithAuthorization {
        from: payer,
        to: r402_evm::batch_settlement::payload::ERC3009_DEPOSIT_COLLECTOR_ADDRESS,
        value: U256::from(50_u64),
        validAfter: U256::ZERO,
        validBefore: U256::from(now + 3_600),
        nonce,
    };
    let sig = signer
        .sign_hash(&typed.eip712_signing_hash(&domain))
        .await
        .expect("receive auth");
    let sig_bytes = Bytes::from(sig.as_bytes().to_vec());
    let payload = BatchSettlementPayload::Deposit {
        channel_config: config,
        voucher,
        deposit: Box::new(DepositBody {
            amount: TokenAmount::from(U256::from(50_u64)),
            authorization: DepositAuthorization::Erc3009 {
                erc3009_authorization: DepositErc3009Authorization {
                    valid_after: TokenAmount::from(U256::ZERO),
                    valid_before: TokenAmount::from(U256::from(now + 3_600)),
                    salt,
                    signature: sig_bytes.clone(),
                },
            },
        }),
    };
    (payload, config, sig_bytes)
}

fn assert_success_hash(resp: SettleResponse) {
    match resp {
        SettleResponse::Success { transaction, .. } => {
            assert!(!transaction.is_empty(), "on-chain success must emit a hash");
            assert_eq!(transaction.as_str(), FIXED_HASH.to_string());
        }
        other => panic!("expected Success, got {other:?}"),
    }
}

fn first_selector(sent: &Mutex<Vec<Bytes>>) -> [u8; 4] {
    let guard = sent.lock().unwrap();
    let data = guard.first().expect("broadcast calldata");
    assert!(data.len() >= 4, "calldata too short");
    [data[0], data[1], data[2], data[3]]
}

#[tokio::test]
async fn deposit_6492_empty_allowlist_is_factory_not_allowed() {
    let (signer, _) = wallet();
    let payer = signer.address();
    let config = sample_config(payer);
    let channel_id = compute_channel_id(&config, 8453).expect("id");
    let voucher = sign_voucher(&signer, channel_id, U256::from(50_u64), 8453)
        .await
        .expect("voucher");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let wrapped = wrap_6492(Address::repeat_byte(0xF1), Bytes::from(vec![0u8; 65]));
    let payload = BatchSettlementPayload::Deposit {
        channel_config: config,
        voucher,
        deposit: Box::new(DepositBody {
            amount: TokenAmount::from(U256::from(50_u64)),
            authorization: DepositAuthorization::Erc3009 {
                erc3009_authorization: DepositErc3009Authorization {
                    valid_after: TokenAmount::from(U256::ZERO),
                    valid_before: TokenAmount::from(U256::from(now + 3_600)),
                    salt: B256::repeat_byte(0xab),
                    signature: wrapped,
                },
            },
        }),
    };
    let req = settle_request(payload, requirements_for(&config));
    let (fac, _) = recording_fac().await;
    let resp = fac.settle(req).await.expect("structured failure");
    match resp {
        SettleResponse::Failure {
            reason,
            transaction,
            ..
        } => {
            assert_eq!(reason.as_str(), EIP6492_FACTORY_NOT_ALLOWED);
            assert!(transaction.is_empty());
        }
        other => panic!("expected Failure, got {other:?}"),
    }
}

#[tokio::test]
async fn claim_settle_broadcasts_claim_with_signature_selector() {
    let (fac, sent) = recording_fac().await;
    let (signer, _) = wallet();
    let config = sample_config(signer.address());
    let payload = BatchSettlementPayload::Claim {
        claims: vec![VoucherClaim {
            voucher: ClaimVoucherBody {
                channel: config,
                max_claimable_amount: TokenAmount::from(U256::from(50_u64)),
            },
            signature: Bytes::from(vec![0u8; 65]),
            total_claimed: TokenAmount::from(U256::from(50_u64)),
        }],
        claim_authorizer_signature: Some(Bytes::from(vec![0u8; 65])),
    };
    let resp = fac
        .settle(settle_request(payload, requirements_for(&config)))
        .await
        .expect("claim settle");
    assert_success_hash(resp);
    assert_eq!(
        first_selector(&sent),
        sel(
            "claimWithSignature((((address,address,address,address,address,uint40,bytes32),uint128),bytes,uint128)[],bytes)"
        )
    );
}

#[tokio::test]
async fn contract_settle_broadcasts_settle_selector() {
    let (fac, sent) = recording_fac().await;
    let config = sample_config(Address::repeat_byte(0x11));
    let payload = BatchSettlementPayload::Settle {
        receiver: config.receiver,
        token: config.token,
    };
    let resp = fac
        .settle(settle_request(payload, requirements_for(&config)))
        .await
        .expect("contract settle");
    assert_success_hash(resp);
    assert_eq!(first_selector(&sent), sel("settle(address,address)"));
}

#[tokio::test]
async fn refund_broadcasts_refund_with_signature_selector() {
    let (fac, sent) = recording_fac().await;
    let (signer, _) = wallet();
    let config = sample_config(signer.address());
    let channel_id = compute_channel_id(&config, 8453).expect("id");
    let voucher = VoucherFields {
        channel_id,
        max_claimable_amount: TokenAmount::from(U256::ZERO),
        signature: Bytes::from(vec![0u8; 65]),
    };
    let payload = BatchSettlementPayload::Refund {
        channel_config: config,
        voucher,
        amount: Some(TokenAmount::from(U256::from(10_u64))),
        refund_nonce: Some(TokenAmount::from(U256::ZERO)),
        claims: Some(Vec::new()),
        refund_authorizer_signature: Some(Bytes::from(vec![0u8; 65])),
        claim_authorizer_signature: None,
    };
    let resp = fac
        .settle(settle_request(payload, requirements_for(&config)))
        .await
        .expect("refund settle");
    assert_success_hash(resp);
    assert_eq!(
        first_selector(&sent),
        sel(
            "refundWithSignature((address,address,address,address,address,uint40,bytes32),uint128,uint256,bytes)"
        )
    );
}

#[tokio::test]
async fn deposit_broadcasts_deposit_selector() {
    let (fac, sent) = recording_fac().await;
    let (signer, _) = wallet();
    let (payload, config, _) = signed_deposit(&signer).await;
    let resp = fac
        .settle(settle_request(payload, requirements_for(&config)))
        .await
        .expect("deposit settle");
    assert_success_hash(resp);
    assert_eq!(
        first_selector(&sent),
        sel(
            "deposit((address,address,address,address,address,uint40,bytes32),uint128,address,bytes)"
        )
    );
}

#[tokio::test]
async fn unenriched_refund_settle_is_invalid_payload_type() {
    let (signer, _) = wallet();
    let config = sample_config(signer.address());
    let channel_id = compute_channel_id(&config, 8453).expect("id");
    let voucher = sign_voucher(&signer, channel_id, U256::ZERO, 8453)
        .await
        .expect("voucher");
    let payload = BatchSettlementPayload::Refund {
        channel_config: config,
        voucher,
        amount: Some(TokenAmount::from(U256::from(10_u64))),
        refund_nonce: None,
        claims: None,
        refund_authorizer_signature: None,
        claim_authorizer_signature: None,
    };
    let fac = Eip155BatchSettlementFacilitator::try_new(dummy_provider()).expect("try_new");
    let resp = fac
        .settle(settle_request(payload, requirements_for(&config)))
        .await
        .expect("structured failure");
    match resp {
        SettleResponse::Failure {
            reason,
            transaction,
            ..
        } => {
            assert_eq!(reason.as_str(), INVALID_PAYLOAD_TYPE);
            assert!(transaction.is_empty());
        }
        other => panic!("expected Failure, got {other:?}"),
    }
}

#[tokio::test]
async fn deposit_7702_owner_ecdsa_is_eip1271_not_eoa() {
    let h = harness().await;
    *h.rpc.payer_code.lock().unwrap() = vec![0xef, 0x01];
    let (signer, _) = wallet();
    let (payload, config, _) = signed_deposit(&signer).await;
    let err = h
        .fac
        .verify(verify_request(payload, requirements_for(&config)))
        .await
        .expect_err("7702 code takes EIP-1271");
    assert_eq!(verify_reason(err), INVALID_RECEIVE_AUTHORIZATION_SIGNATURE);
}

#[tokio::test]
async fn permit2_witness_mismatch_is_channel_id_mismatch() {
    let (fac, _) = recording_fac().await;
    let (signer, _) = wallet();
    let config = sample_config(signer.address());
    let channel_id = compute_channel_id(&config, 8453).expect("id");
    let voucher = sign_voucher(&signer, channel_id, U256::from(50_u64), 8453)
        .await
        .expect("voucher");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let payload = BatchSettlementPayload::Deposit {
        channel_config: config,
        voucher,
        deposit: Box::new(DepositBody {
            amount: TokenAmount::from(U256::from(50_u64)),
            authorization: DepositAuthorization::Permit2 {
                permit2_authorization: DepositPermit2Authorization {
                    from: signer.address(),
                    permitted: DepositTokenPermissions {
                        token: config.token,
                        amount: TokenAmount::from(U256::from(50_u64)),
                    },
                    spender: PERMIT2_DEPOSIT_COLLECTOR_ADDRESS,
                    nonce: TokenAmount::from(U256::ZERO),
                    deadline: TokenAmount::from(U256::from(now + 3_600)),
                    witness: DepositWitness {
                        channel_id: B256::repeat_byte(0xff),
                    },
                    signature: Bytes::from(vec![0u8; 65]),
                },
            },
        }),
    };
    let err = fac
        .verify(verify_request(payload, requirements_for(&config)))
        .await
        .expect_err("witness mismatch");
    assert_eq!(verify_reason(err), CHANNEL_ID_MISMATCH);
}

#[tokio::test]
async fn permit2_allowance_rpc_err_is_allowance_required() {
    let h = harness().await;
    *h.rpc.allowance.lock().unwrap() = None;
    let (signer, _) = wallet();
    let config = sample_config(signer.address());
    let channel_id = compute_channel_id(&config, 8453).expect("id");
    let voucher = sign_voucher(&signer, channel_id, U256::from(50_u64), 8453)
        .await
        .expect("voucher");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let domain = eip712_domain! {
        name: "Permit2",
        chain_id: 8453_u64,
        verifying_contract: PERMIT2_ADDRESS,
    };
    let typed = permit2_td::PermitWitnessTransferFrom {
        permitted: permit2_td::TokenPermissions {
            token: config.token,
            amount: U256::from(50_u64),
        },
        spender: PERMIT2_DEPOSIT_COLLECTOR_ADDRESS,
        nonce: U256::ZERO,
        deadline: U256::from(now + 3_600),
        witness: permit2_td::DepositWitness {
            channelId: channel_id,
        },
    };
    let sig = signer
        .sign_hash(&typed.eip712_signing_hash(&domain))
        .await
        .expect("permit2");
    let payload = BatchSettlementPayload::Deposit {
        channel_config: config,
        voucher,
        deposit: Box::new(DepositBody {
            amount: TokenAmount::from(U256::from(50_u64)),
            authorization: DepositAuthorization::Permit2 {
                permit2_authorization: DepositPermit2Authorization {
                    from: signer.address(),
                    permitted: DepositTokenPermissions {
                        token: config.token,
                        amount: TokenAmount::from(U256::from(50_u64)),
                    },
                    spender: PERMIT2_DEPOSIT_COLLECTOR_ADDRESS,
                    nonce: TokenAmount::from(U256::ZERO),
                    deadline: TokenAmount::from(U256::from(now + 3_600)),
                    witness: DepositWitness { channel_id },
                    signature: Bytes::from(sig.as_bytes().to_vec()),
                },
            },
        }),
    };
    let err = h
        .fac
        .verify(verify_request(payload, requirements_for(&config)))
        .await
        .expect_err("allowance rpc fail-closed");
    assert_eq!(verify_reason(err), PERMIT2_ALLOWANCE_REQUIRED);
}

#[tokio::test]
async fn refund_verify_zero_balance_is_channel_not_found() {
    let h = harness().await;
    *h.rpc.channel_balance.lock().unwrap() = 0;
    let (signer, _) = wallet();
    let config = sample_config(signer.address());
    let channel_id = compute_channel_id(&config, 8453).expect("id");
    let voucher = sign_voucher(&signer, channel_id, U256::from(50_u64), 8453)
        .await
        .expect("voucher");
    let payload = BatchSettlementPayload::Refund {
        channel_config: config,
        voucher,
        amount: Some(TokenAmount::from(U256::from(10_u64))),
        refund_nonce: None,
        claims: None,
        refund_authorizer_signature: None,
        claim_authorizer_signature: None,
    };
    let err = h
        .fac
        .verify(verify_request(payload, requirements_for(&config)))
        .await
        .expect_err("empty channel");
    assert_eq!(verify_reason(err), CHANNEL_NOT_FOUND);
}

#[tokio::test]
async fn deposit_pending_receipt_wait_then_reconcile() {
    use alloy_primitives::hex;
    let h = harness().await;
    *h.send_mode.lock().unwrap() = SendMode::ReceiptWait;
    let (signer, _) = wallet();
    let (payload, config, sig) = signed_deposit(&signer).await;
    let key = hex::encode_prefixed(&sig);
    let req = settle_request(payload.clone(), requirements_for(&config));
    let first = h.fac.settle(req).await.expect("pending");
    match first {
        SettleResponse::Failure {
            reason,
            transaction,
            ..
        } => {
            assert_eq!(reason.as_str(), "settlement_pending");
            assert_eq!(transaction.as_str(), FIXED_HASH.to_string());
        }
        other => panic!("expected settlement_pending, got {other:?}"),
    }
    assert_eq!(
        h.pending.get(&key).as_deref(),
        Some(FIXED_HASH.to_string().as_str())
    );
    assert_eq!(h.sent.lock().unwrap().len(), 1);

    *h.rpc.mined.lock().unwrap() = true;
    let second = h
        .fac
        .settle(settle_request(payload, requirements_for(&config)))
        .await
        .expect("reconcile");
    assert_success_hash(second);
    assert_eq!(
        h.sent.lock().unwrap().len(),
        1,
        "retry must not re-broadcast"
    );
    assert!(h.pending.get(&key).is_none());
}

#[tokio::test]
async fn deposit_pending_unconfirmed_does_not_rebroadcast() {
    use alloy_primitives::hex;
    let h = harness().await;
    *h.send_mode.lock().unwrap() = SendMode::ReceiptWait;
    let (signer, _) = wallet();
    let (payload, config, sig) = signed_deposit(&signer).await;
    let key = hex::encode_prefixed(&sig);
    let _ = h
        .fac
        .settle(settle_request(payload.clone(), requirements_for(&config)))
        .await
        .expect("pending");
    assert_eq!(h.sent.lock().unwrap().len(), 1);
    let retry = h
        .fac
        .settle(settle_request(payload, requirements_for(&config)))
        .await
        .expect("still pending");
    match retry {
        SettleResponse::Failure {
            reason,
            transaction,
            ..
        } => {
            assert_eq!(reason.as_str(), "settlement_pending");
            assert_eq!(transaction.as_str(), FIXED_HASH.to_string());
        }
        other => panic!("expected settlement_pending, got {other:?}"),
    }
    assert_eq!(h.sent.lock().unwrap().len(), 1);
    assert_eq!(
        h.pending.get(&key).as_deref(),
        Some(FIXED_HASH.to_string().as_str())
    );
}
