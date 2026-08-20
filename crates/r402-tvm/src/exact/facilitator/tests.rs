//! Unit tests for TON exact verify/settle.

#![cfg(feature = "client")]
#![allow(
    clippy::panic,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unused_async,
    reason = "test assertions"
)]

use std::time::{SystemTime, UNIX_EPOCH};

use r402_core::cache::SettlementCache;
use r402_core::wire::VerifyResponse;
use serde_json::{Value, json};

use super::verify_response_json;
use crate::chain::rpc::{TvmAccountState, TvmJettonWalletData, TvmRpc, TvmRpcError};
use crate::chain::{TvmAddress, TvmRelayRequest};
use crate::codecs::common::cell_hash_base64;
use crate::codecs::jetton::build_jetton_transfer_body;
use crate::codecs::w5::{parse_exact_tvm_payload, parse_w5_init_data};
use crate::exact::client::TvmW5Signer;
use crate::exact::error::{
    ERR_EXACT_TVM_FACILITATOR_INSUFFICIENT_BALANCE, ERR_EXACT_TVM_INSUFFICIENT_BALANCE,
    ERR_EXACT_TVM_INVALID_AMOUNT, ERR_EXACT_TVM_INVALID_ASSET, ERR_EXACT_TVM_INVALID_CODE_HASH,
    ERR_EXACT_TVM_INVALID_RECIPIENT, ERR_EXACT_TVM_INVALID_SEQNO, ERR_EXACT_TVM_INVALID_WALLET_ID,
    ERR_EXACT_TVM_SIMULATION_FAILED, ERR_EXACT_TVM_UNSUPPORTED_NETWORK,
    ERR_EXACT_TVM_UNSUPPORTED_SCHEME, ERR_EXACT_TVM_UNSUPPORTED_VERSION,
};
use crate::exact::types::TvmExtra;
use crate::{
    DEFAULT_JETTON_WALLET_MESSAGE_AMOUNT, MIN_FACILITATOR_TON_BALANCE, USDT_TESTNET_MINTER,
    W5R1_CODE_HEX,
};

const PAY_TO: &str = "0:2222222222222222222222222222222222222222222222222222222222222222";
const SOURCE_JETTON_WALLET: &str =
    "0:3333333333333333333333333333333333333333333333333333333333333333";
const FACILITATOR: &str = "0:4444444444444444444444444444444444444444444444444444444444444444";

#[derive(Clone)]
struct MockRpc {
    facilitator_balance: u128,
    jetton_balance: u128,
    payer_seqno: Option<u32>,
    payer_wallet_id: Option<u32>,
    payer_bad_code: bool,
}

impl Default for MockRpc {
    fn default() -> Self {
        Self {
            facilitator_balance: MIN_FACILITATOR_TON_BALANCE + 1,
            jetton_balance: 10_000,
            payer_seqno: None,
            payer_wallet_id: None,
            payer_bad_code: false,
        }
    }
}

impl TvmRpc for MockRpc {
    async fn get_account_state(&self, address: &str) -> Result<TvmAccountState, TvmRpcError> {
        let addr: TvmAddress = address
            .parse()
            .map_err(|e: crate::chain::TvmAddressFormatError| TvmRpcError::Parse(e.to_string()))?;
        if address == FACILITATOR {
            return Ok(TvmAccountState {
                address: addr,
                balance: self.facilitator_balance,
                is_active: true,
                is_uninitialized: false,
                is_frozen: false,
                state_init: None,
            });
        }
        if self.payer_seqno.is_some() || self.payer_wallet_id.is_some() || self.payer_bad_code {
            let signer = fixture_signer();
            let mut init = signer_state_init(&signer);
            if self.payer_bad_code {
                init.code = tonlib_core::cell::CellBuilder::new()
                    .build()
                    .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
            }
            if let Some(seqno) = self.payer_seqno {
                let parsed =
                    parse_w5_init_data(&init).map_err(|e| TvmRpcError::Parse(e.to_string()))?;
                init.data = tonlib_core::cell::CellBuilder::new()
                    .store_bit(parsed.signature_allowed)
                    .and_then(|b| b.store_u32(32, seqno))
                    .and_then(|b| b.store_u32(32, self.payer_wallet_id.unwrap_or(parsed.wallet_id)))
                    .and_then(|b| b.store_slice(&parsed.public_key))
                    .and_then(|b| b.store_bit(false))
                    .and_then(tonlib_core::cell::CellBuilder::build)
                    .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
            }
            return Ok(TvmAccountState {
                address: addr,
                balance: 0,
                is_active: true,
                is_uninitialized: false,
                is_frozen: false,
                state_init: Some(init),
            });
        }
        Ok(TvmAccountState {
            address: addr,
            balance: 0,
            is_active: false,
            is_uninitialized: true,
            is_frozen: false,
            state_init: None,
        })
    }

    async fn get_jetton_wallet(
        &self,
        _asset: &str,
        _owner: &str,
    ) -> Result<TvmAddress, TvmRpcError> {
        SOURCE_JETTON_WALLET
            .parse()
            .map_err(|e: crate::chain::TvmAddressFormatError| TvmRpcError::Parse(e.to_string()))
    }

    async fn get_jetton_wallet_data(
        &self,
        address: &str,
    ) -> Result<TvmJettonWalletData, TvmRpcError> {
        Ok(TvmJettonWalletData {
            address: address
                .parse()
                .map_err(|e: crate::chain::TvmAddressFormatError| {
                    TvmRpcError::Parse(e.to_string())
                })?,
            balance: self.jetton_balance,
            owner: fixture_signer().address().clone(),
            jetton_minter: USDT_TESTNET_MINTER.parse().map_err(
                |e: crate::chain::TvmAddressFormatError| TvmRpcError::Parse(e.to_string()),
            )?,
        })
    }

    async fn emulate_trace(
        &self,
        _boc: &[u8],
        _ignore: bool,
        _timeout: u64,
    ) -> Result<Value, TvmRpcError> {
        Ok(json!({ "transactions": {} }))
    }

    async fn send_message(&self, _boc: &[u8]) -> Result<String, TvmRpcError> {
        Ok("external-hash".to_owned())
    }

    async fn get_trace_by_message_hash(&self, _hash: &str) -> Result<Value, TvmRpcError> {
        Ok(json!({ "transactions": {} }))
    }
}

fn fixture_signer() -> TvmW5Signer {
    TvmW5Signer::from_seed(&[7u8; 32], crate::chain::TvmChainReference::TESTNET).unwrap()
}

fn signer_state_init(signer: &TvmW5Signer) -> crate::codecs::w5::StateInitCells {
    crate::codecs::w5::build_w5r1_state_init(
        &{
            // public key is inside state init data; rebuild from seed
            let s = TvmW5Signer::from_seed(&[7u8; 32], crate::chain::TvmChainReference::TESTNET)
                .unwrap();
            let _ = s;
            ed25519_dalek::SigningKey::from_bytes(&[7u8; 32])
                .verifying_key()
                .to_bytes()
        },
        signer.wallet_id(),
    )
    .unwrap()
}

fn fixture_request(
    amount: &str,
    asset: &str,
    pay_to: &str,
    version: u64,
    scheme: &str,
) -> (Value, TvmW5Signer, String) {
    let signer = fixture_signer();
    let extra = TvmExtra::sponsored();
    let pay_to_addr: TvmAddress = pay_to.parse().unwrap();
    let body = build_jetton_transfer_body(amount.parse().unwrap(), &pay_to_addr, &extra).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let boc = signer
        .sign_transfer(
            0,
            u32::try_from(now + 60).unwrap(),
            &SOURCE_JETTON_WALLET.parse().unwrap(),
            DEFAULT_JETTON_WALLET_MESSAGE_AMOUNT,
            &body,
            true,
        )
        .unwrap();
    let payer = signer.address().to_string();
    let req = json!({
        "paymentPayload": {
            "x402Version": version,
            "accepted": {
                "scheme": scheme,
                "network": "tvm:-3",
                "amount": amount,
                "asset": asset,
                "payTo": pay_to,
                "maxTimeoutSeconds": 300,
                "extra": { "areFeesSponsored": true }
            },
            "payload": {
                "settlementBoc": boc,
                "asset": asset
            }
        },
        "paymentRequirements": {
            "scheme": "exact",
            "network": "tvm:-3",
            "amount": amount,
            "asset": asset,
            "payTo": pay_to,
            "maxTimeoutSeconds": 300,
            "extra": { "areFeesSponsored": true }
        }
    });
    (req, signer, payer)
}

fn good_trace(req: &Value) -> Value {
    let boc = req["paymentPayload"]["payload"]["settlementBoc"]
        .as_str()
        .unwrap();
    let settlement = parse_exact_tvm_payload(boc).unwrap();
    json!({
        "transactions": {
            "payer": {
                "account": settlement.payer.as_str(),
                "hash": "a".repeat(64),
                "hash_norm": "b".repeat(64),
                "description": {
                    "aborted": false,
                    "compute_ph": { "skipped": false, "success": true, "gas_fees": "100" },
                    "action": { "success": true, "fwd_fee": "10" },
                    "storage_ph": { "storage_fees_collected": "1", "storage_fees_due": "0" }
                },
                "in_msg": { "message_content": { "hash": cell_hash_base64(&settlement.body) } },
                "out_msgs": [{
                    "destination": settlement.transfer.source_wallet.as_str(),
                    "hash": "out-message-hash",
                    "fwd_fee": "1000",
                    "message_content": {
                        "hash": crate::codecs::common::cell_hash_base64_from_ton_hash(&settlement.transfer.body_hash)
                    }
                }]
            },
            "sourceWallet": {
                "account": settlement.transfer.source_wallet.as_str(),
                "hash": "c".repeat(64),
                "description": {
                    "aborted": false,
                    "compute_ph": { "skipped": false, "success": true },
                    "action": { "success": true }
                },
                "in_msg": { "hash": "out-message-hash" }
            }
        }
    })
}

async fn emulate_ok(relay: &TvmRelayRequest) -> Result<Value, String> {
    let _ = relay;
    Err("use per-test closure".to_owned())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn reason(resp: &VerifyResponse) -> String {
    match resp {
        VerifyResponse::Invalid { reason, .. } => reason.as_str().to_owned(),
        VerifyResponse::Valid { .. } => "valid".to_owned(),
        _ => "other".to_owned(),
    }
}

#[tokio::test]
async fn verifies_native_w5r1_settlement_boc() {
    let (req, _, payer) = fixture_request("10000", USDT_TESTNET_MINTER, PAY_TO, 2, "exact");
    let rpc = MockRpc::default();
    let trace = good_trace(&req);
    let resp = verify_response_json(
        &rpc,
        &[FACILITATOR.to_owned()],
        &req,
        now(),
        async move |_| Ok(trace.clone()),
    )
    .await;
    assert!(resp.is_valid(), "expected valid, got {resp:?}");
    match resp {
        VerifyResponse::Valid { payer: p, .. } => assert_eq!(p, payer),
        _ => panic!("expected valid"),
    }
}

#[tokio::test]
async fn rejects_unsupported_version() {
    let (req, _, _) = fixture_request("10000", USDT_TESTNET_MINTER, PAY_TO, 1, "exact");
    let rpc = MockRpc::default();
    let resp = verify_response_json(
        &rpc,
        &[FACILITATOR.to_owned()],
        &req,
        now(),
        async move |_| Ok(json!({"transactions":{}})),
    )
    .await;
    assert_eq!(reason(&resp), ERR_EXACT_TVM_UNSUPPORTED_VERSION);
}

#[tokio::test]
async fn rejects_amount_mismatch() {
    let (mut req, _, _) = fixture_request("10000", USDT_TESTNET_MINTER, PAY_TO, 2, "exact");
    req["paymentRequirements"]["amount"] = json!("10001");
    let rpc = MockRpc::default();
    let resp = verify_response_json(
        &rpc,
        &[FACILITATOR.to_owned()],
        &req,
        now(),
        async move |_| Ok(json!({"transactions":{}})),
    )
    .await;
    assert_eq!(reason(&resp), ERR_EXACT_TVM_INVALID_AMOUNT);
}

#[tokio::test]
async fn rejects_unsupported_network() {
    let (mut req, _, _) = fixture_request("10000", USDT_TESTNET_MINTER, PAY_TO, 2, "exact");
    req["paymentRequirements"]["network"] = json!("tvm:999");
    let rpc = MockRpc::default();
    let resp = verify_response_json(
        &rpc,
        &[FACILITATOR.to_owned()],
        &req,
        now(),
        async move |_| Ok(json!({"transactions":{}})),
    )
    .await;
    assert_eq!(reason(&resp), ERR_EXACT_TVM_UNSUPPORTED_NETWORK);
}

#[tokio::test]
async fn rejects_unsupported_scheme() {
    let (req, _, _) = fixture_request("10000", USDT_TESTNET_MINTER, PAY_TO, 2, "upto");
    let rpc = MockRpc::default();
    let resp = verify_response_json(
        &rpc,
        &[FACILITATOR.to_owned()],
        &req,
        now(),
        async move |_| Ok(json!({"transactions":{}})),
    )
    .await;
    assert_eq!(reason(&resp), ERR_EXACT_TVM_UNSUPPORTED_SCHEME);
}

#[tokio::test]
async fn rejects_asset_mismatch() {
    let (mut req, _, _) = fixture_request("10000", USDT_TESTNET_MINTER, PAY_TO, 2, "exact");
    req["paymentRequirements"]["asset"] =
        json!("0:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let rpc = MockRpc::default();
    let resp = verify_response_json(
        &rpc,
        &[FACILITATOR.to_owned()],
        &req,
        now(),
        async move |_| Ok(json!({"transactions":{}})),
    )
    .await;
    assert_eq!(reason(&resp), ERR_EXACT_TVM_INVALID_ASSET);
}

#[tokio::test]
async fn rejects_payee_mismatch() {
    let (mut req, _, _) = fixture_request("10000", USDT_TESTNET_MINTER, PAY_TO, 2, "exact");
    req["paymentRequirements"]["payTo"] =
        json!("0:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    let rpc = MockRpc::default();
    let resp = verify_response_json(
        &rpc,
        &[FACILITATOR.to_owned()],
        &req,
        now(),
        async move |_| Ok(json!({"transactions":{}})),
    )
    .await;
    assert_eq!(reason(&resp), ERR_EXACT_TVM_INVALID_RECIPIENT);
}

#[tokio::test]
async fn rejects_seqno_mismatch() {
    let (req, _, _) = fixture_request("10000", USDT_TESTNET_MINTER, PAY_TO, 2, "exact");
    let rpc = MockRpc {
        payer_seqno: Some(1),
        ..MockRpc::default()
    };
    let resp = verify_response_json(
        &rpc,
        &[FACILITATOR.to_owned()],
        &req,
        now(),
        async move |_| Ok(json!({"transactions":{}})),
    )
    .await;
    assert_eq!(reason(&resp), ERR_EXACT_TVM_INVALID_SEQNO);
}

#[tokio::test]
async fn rejects_wallet_id_mismatch() {
    let (req, _, _) = fixture_request("10000", USDT_TESTNET_MINTER, PAY_TO, 2, "exact");
    let rpc = MockRpc {
        payer_seqno: Some(0),
        payer_wallet_id: Some(123),
        ..MockRpc::default()
    };
    let resp = verify_response_json(
        &rpc,
        &[FACILITATOR.to_owned()],
        &req,
        now(),
        async move |_| Ok(json!({"transactions":{}})),
    )
    .await;
    assert_eq!(reason(&resp), ERR_EXACT_TVM_INVALID_WALLET_ID);
}

#[tokio::test]
async fn rejects_invalid_active_code() {
    let (req, _, _) = fixture_request("10000", USDT_TESTNET_MINTER, PAY_TO, 2, "exact");
    let rpc = MockRpc {
        payer_bad_code: true,
        ..MockRpc::default()
    };
    let resp = verify_response_json(
        &rpc,
        &[FACILITATOR.to_owned()],
        &req,
        now(),
        async move |_| Ok(json!({"transactions":{}})),
    )
    .await;
    assert_eq!(reason(&resp), ERR_EXACT_TVM_INVALID_CODE_HASH);
}

#[tokio::test]
async fn rejects_insufficient_jetton_balance() {
    let (req, _, _) = fixture_request("10000", USDT_TESTNET_MINTER, PAY_TO, 2, "exact");
    let rpc = MockRpc {
        jetton_balance: 9999,
        ..MockRpc::default()
    };
    let resp = verify_response_json(
        &rpc,
        &[FACILITATOR.to_owned()],
        &req,
        now(),
        async move |_| Ok(json!({"transactions":{}})),
    )
    .await;
    assert_eq!(reason(&resp), ERR_EXACT_TVM_INSUFFICIENT_BALANCE);
}

#[tokio::test]
async fn rejects_insufficient_facilitator_ton() {
    let (req, _, _) = fixture_request("10000", USDT_TESTNET_MINTER, PAY_TO, 2, "exact");
    let rpc = MockRpc {
        facilitator_balance: MIN_FACILITATOR_TON_BALANCE - 1,
        ..MockRpc::default()
    };
    let resp = verify_response_json(
        &rpc,
        &[FACILITATOR.to_owned()],
        &req,
        now(),
        async move |_| Ok(json!({"transactions":{}})),
    )
    .await;
    assert_eq!(
        reason(&resp),
        ERR_EXACT_TVM_FACILITATOR_INSUFFICIENT_BALANCE
    );
}

#[tokio::test]
async fn rejects_simulation_failure() {
    let (req, _, _) = fixture_request("10000", USDT_TESTNET_MINTER, PAY_TO, 2, "exact");
    let rpc = MockRpc::default();
    let resp = verify_response_json(
        &rpc,
        &[FACILITATOR.to_owned()],
        &req,
        now(),
        async move |_| Ok(json!({ "transactions": {} })),
    )
    .await;
    assert_eq!(reason(&resp), ERR_EXACT_TVM_SIMULATION_FAILED);
}

#[tokio::test]
async fn cache_reserve_blocks_duplicate_without_release() {
    let cache = SettlementCache::new();
    assert_eq!(cache.reserve("abc"), r402_core::cache::Duplicate::No);
    assert_eq!(cache.reserve("abc"), r402_core::cache::Duplicate::Yes);
}

#[test]
fn w5r1_code_hex_is_loaded() {
    let _ = W5R1_CODE_HEX;
    assert!(!W5R1_CODE_HEX.is_empty());
}

#[test]
fn unused_emulate_ok_compiles() {
    let _ = emulate_ok;
}
