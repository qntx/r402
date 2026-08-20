//! Unit tests for Algorand exact verify/settle.

#![allow(
    clippy::panic,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    reason = "test assertions with known JSON structure"
)]

use base64::Engine;
use r402_core::cache::SettlementCache;
use r402_core::error::ErrorReason;
use r402_core::wire::{SettleResponse, VerifyResponse};
use serde_json::{Value, json};

use super::{settle_request, verify_request_json};
use crate::USDC_TESTNET_ASA_ID;
use crate::chain::codec::{
    SignedTransaction, Transaction, TxnType, assign_group, encode_signed, encode_txn,
};
use crate::chain::rpc::{
    AlgodError, AlgodRpc, PendingTransaction, SimulateResult, SuggestedParams,
};
use crate::chain::signer::AlgorandSigner;
use crate::chain::types::{AlgorandAddress, AlgorandChainReference};
use crate::exact::error::invalid;
use crate::exact::types::ExactAvmPayload;

fn seed(n: u8) -> AlgorandSigner {
    AlgorandSigner::from_seed([n; 32])
}

#[derive(Clone)]
struct MockRpc {
    params: SuggestedParams,
    simulate: SimulateResult,
    send_ok: bool,
    pending: PendingTransaction,
}

impl Default for MockRpc {
    fn default() -> Self {
        Self {
            params: SuggestedParams {
                fee_per_byte: 0,
                min_fee: 1000,
                genesis_hash: AlgorandChainReference::TESTNET.genesis_hash(),
                genesis_id: AlgorandChainReference::TESTNET.genesis_id().to_owned(),
                last_round: 1_000,
            },
            simulate: SimulateResult {
                failure_message: None,
            },
            send_ok: true,
            pending: PendingTransaction {
                confirmed_round: Some(1_001),
                pool_error: String::new(),
            },
        }
    }
}

impl AlgodRpc for MockRpc {
    async fn suggested_params(&self) -> Result<SuggestedParams, AlgodError> {
        Ok(self.params.clone())
    }

    async fn simulate_group(&self, _signed_txns: &[Vec<u8>]) -> Result<SimulateResult, AlgodError> {
        Ok(self.simulate.clone())
    }

    async fn send_group(&self, _signed_txns: &[Vec<u8>]) -> Result<String, AlgodError> {
        if self.send_ok {
            Ok("SENT".to_owned())
        } else {
            Err(AlgodError::Http("broadcast failed".to_owned()))
        }
    }

    async fn pending_transaction(&self, _txid: &str) -> Result<PendingTransaction, AlgodError> {
        Ok(self.pending.clone())
    }

    async fn last_round(&self) -> Result<u64, AlgodError> {
        Ok(self.params.last_round)
    }
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn signed_group(
    client: &AlgorandSigner,
    fee_payer: &AlgorandSigner,
    pay_to: AlgorandAddress,
    amount: u64,
    asset_id: u64,
    fee_payer_fee: u64,
) -> ExactAvmPayload {
    signed_group_on(
        AlgorandChainReference::TESTNET,
        client,
        fee_payer,
        pay_to,
        amount,
        asset_id,
        fee_payer_fee,
    )
}

fn signed_group_on(
    chain: AlgorandChainReference,
    client: &AlgorandSigner,
    fee_payer: &AlgorandSigner,
    pay_to: AlgorandAddress,
    amount: u64,
    asset_id: u64,
    fee_payer_fee: u64,
) -> ExactAvmPayload {
    let mut pay = Transaction::new(TxnType::Pay, fee_payer.address());
    pay.receiver = Some(fee_payer.address());
    pay.fee = fee_payer_fee;
    pay.first_valid = 1_000;
    pay.last_valid = 2_000;
    pay.genesis_id = chain.genesis_id().to_owned();
    pay.genesis_hash = chain.genesis_hash();

    let mut axfer = Transaction::new(TxnType::Axfer, client.address());
    axfer.asset_id = asset_id;
    axfer.asset_amount = amount;
    axfer.asset_receiver = Some(pay_to);
    axfer.first_valid = 1_000;
    axfer.last_valid = 2_000;
    axfer.genesis_id = chain.genesis_id().to_owned();
    axfer.genesis_hash = chain.genesis_hash();

    let grouped = assign_group(vec![pay, axfer]);
    let grouped_pay = grouped[0].clone();
    let grouped_axfer = grouped[1].clone();
    let unsigned_pay = encode_signed(&SignedTransaction {
        sig: None,
        txn: grouped_pay,
    });
    let signed_axfer = encode_signed(&client.sign(&grouped_axfer));
    ExactAvmPayload {
        payment_group: vec![b64(&unsigned_pay), b64(&signed_axfer)],
        payment_index: 1,
    }
}

fn signed_group_mut_axfer(
    client: &AlgorandSigner,
    fee_payer: &AlgorandSigner,
    pay_to: AlgorandAddress,
    mutate: impl FnOnce(&mut Transaction),
) -> ExactAvmPayload {
    let mut pay = Transaction::new(TxnType::Pay, fee_payer.address());
    pay.receiver = Some(fee_payer.address());
    pay.fee = 2000;
    pay.first_valid = 1_000;
    pay.last_valid = 2_000;
    pay.genesis_id = AlgorandChainReference::TESTNET.genesis_id().to_owned();
    pay.genesis_hash = AlgorandChainReference::TESTNET.genesis_hash();

    let mut axfer = Transaction::new(TxnType::Axfer, client.address());
    axfer.asset_id = USDC_TESTNET_ASA_ID;
    axfer.asset_amount = 1_000_000;
    axfer.asset_receiver = Some(pay_to);
    axfer.first_valid = 1_000;
    axfer.last_valid = 2_000;
    axfer.genesis_id = AlgorandChainReference::TESTNET.genesis_id().to_owned();
    axfer.genesis_hash = AlgorandChainReference::TESTNET.genesis_hash();
    mutate(&mut axfer);

    let grouped = assign_group(vec![pay, axfer]);
    let unsigned_pay = encode_signed(&SignedTransaction {
        sig: None,
        txn: grouped[0].clone(),
    });
    let signed_axfer = encode_signed(&client.sign(&grouped[1]));
    ExactAvmPayload {
        payment_group: vec![b64(&unsigned_pay), b64(&signed_axfer)],
        payment_index: 1,
    }
}

fn signed_payment_only(client: &AlgorandSigner, pay_to: AlgorandAddress) -> ExactAvmPayload {
    let mut axfer = Transaction::new(TxnType::Axfer, client.address());
    axfer.asset_id = USDC_TESTNET_ASA_ID;
    axfer.asset_amount = 1_000_000;
    axfer.asset_receiver = Some(pay_to);
    axfer.first_valid = 1_000;
    axfer.last_valid = 2_000;
    axfer.genesis_id = AlgorandChainReference::TESTNET.genesis_id().to_owned();
    axfer.genesis_hash = AlgorandChainReference::TESTNET.genesis_hash();
    let signed_axfer = encode_signed(&client.sign(&axfer));
    ExactAvmPayload {
        payment_group: vec![b64(&signed_axfer)],
        payment_index: 0,
    }
}

fn request_json(
    payload: &ExactAvmPayload,
    pay_to: &AlgorandAddress,
    amount: &str,
    asset: &str,
) -> Value {
    let network = "algorand:SGO1GKSzyE7IEPItTxCByw9x8FmnrCDe";
    json!({
        "x402Version": 2,
        "paymentPayload": {
            "x402Version": 2,
            "accepted": {
                "scheme": "exact",
                "network": network,
                "amount": amount,
                "payTo": pay_to.to_string(),
                "maxTimeoutSeconds": 60,
                "asset": asset,
            },
            "payload": payload,
        },
        "paymentRequirements": {
            "scheme": "exact",
            "network": network,
            "amount": amount,
            "payTo": pay_to.to_string(),
            "maxTimeoutSeconds": 60,
            "asset": asset,
        }
    })
}

fn reason(resp: &VerifyResponse) -> String {
    match resp {
        VerifyResponse::Invalid { reason, .. } => reason.as_str().to_owned(),
        VerifyResponse::Valid { .. } => "valid".to_owned(),
        _ => "other".to_owned(),
    }
}

fn sign_with(
    signer: &AlgorandSigner,
) -> impl FnMut(&Transaction, &str) -> Result<Vec<u8>, crate::exact::error::AlgorandInvalid> + '_ {
    move |txn, sender| {
        if signer.address().to_string() != sender {
            return Err(invalid("invalid_exact_avm_invalid_fee_payer"));
        }
        Ok(encode_signed(&signer.sign(txn)))
    }
}

#[tokio::test]
async fn verify_valid_sponsored_group() {
    let client = seed(1);
    let fee_payer = seed(2);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let payload = signed_group(
        &client,
        &fee_payer,
        pay_to,
        1_000_000,
        USDC_TESTNET_ASA_ID,
        2000,
    );
    let req = request_json(
        &payload,
        &pay_to,
        "1000000",
        &USDC_TESTNET_ASA_ID.to_string(),
    );
    let addrs = vec![fee_payer.address().to_string()];
    let resp = verify_request_json(&MockRpc::default(), &addrs, &req, sign_with(&fee_payer)).await;
    assert!(resp.is_valid(), "expected valid, got {}", reason(&resp));
}

#[tokio::test]
async fn verify_rejects_wrong_version() {
    let mut req = request_json(
        &ExactAvmPayload {
            payment_group: vec!["AA==".to_owned()],
            payment_index: 0,
        },
        &AlgorandAddress::ZERO,
        "1",
        "1",
    );
    req["paymentPayload"]["x402Version"] = json!(1);
    let resp = verify_request_json(&MockRpc::default(), &[], &req, |_, _| {
        Err(invalid("invalid_exact_avm_invalid_fee_payer"))
    })
    .await;
    assert_eq!(reason(&resp), "invalid_exact_avm_invalid_version");
}

#[tokio::test]
async fn verify_rejects_wrong_scheme() {
    let mut req = request_json(
        &ExactAvmPayload {
            payment_group: vec!["AA==".to_owned()],
            payment_index: 0,
        },
        &AlgorandAddress::ZERO,
        "1",
        "1",
    );
    req["paymentPayload"]["accepted"]["scheme"] = json!("upto");
    let resp = verify_request_json(&MockRpc::default(), &[], &req, |_, _| {
        Err(invalid("invalid_exact_avm_invalid_fee_payer"))
    })
    .await;
    assert_eq!(reason(&resp), "invalid_exact_avm_scheme");
}

#[tokio::test]
async fn verify_rejects_network_mismatch() {
    let mut req = request_json(
        &ExactAvmPayload {
            payment_group: vec!["AA==".to_owned()],
            payment_index: 0,
        },
        &AlgorandAddress::ZERO,
        "1",
        "1",
    );
    req["paymentPayload"]["accepted"]["network"] =
        json!("algorand:wGHE2Pwdvd7S12BL5FaOP20EGYesN73k");
    let resp = verify_request_json(&MockRpc::default(), &[], &req, |_, _| {
        Err(invalid("invalid_exact_avm_invalid_fee_payer"))
    })
    .await;
    assert_eq!(reason(&resp), "invalid_exact_avm_network_mismatch");
}

#[tokio::test]
async fn verify_rejects_oversized_group() {
    let group: Vec<String> = (0..17).map(|_| "AA==".to_owned()).collect();
    let req = request_json(
        &ExactAvmPayload {
            payment_group: group,
            payment_index: 0,
        },
        &AlgorandAddress::ZERO,
        "1",
        "1",
    );
    let resp = verify_request_json(&MockRpc::default(), &[], &req, |_, _| {
        Err(invalid("invalid_exact_avm_invalid_fee_payer"))
    })
    .await;
    assert_eq!(reason(&resp), "invalid_exact_avm_group_size_exceeded");
}

#[tokio::test]
async fn verify_rejects_payment_index_oob() {
    let req = request_json(
        &ExactAvmPayload {
            payment_group: vec!["AA==".to_owned()],
            payment_index: 5,
        },
        &AlgorandAddress::ZERO,
        "1",
        "1",
    );
    let resp = verify_request_json(&MockRpc::default(), &[], &req, |_, _| {
        Err(invalid("invalid_exact_avm_invalid_fee_payer"))
    })
    .await;
    assert_eq!(reason(&resp), "invalid_exact_avm_payment_index");
}

#[tokio::test]
async fn verify_rejects_facilitator_as_payer() {
    let fee_payer = seed(2);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let payload = signed_group(
        &fee_payer,
        &fee_payer,
        pay_to,
        1_000_000,
        USDC_TESTNET_ASA_ID,
        2000,
    );
    let req = request_json(
        &payload,
        &pay_to,
        "1000000",
        &USDC_TESTNET_ASA_ID.to_string(),
    );
    let addrs = vec![fee_payer.address().to_string()];
    let resp = verify_request_json(&MockRpc::default(), &addrs, &req, sign_with(&fee_payer)).await;
    assert_eq!(reason(&resp), "invalid_exact_avm_facilitator_transferring");
}

#[tokio::test]
async fn verify_rejects_amount_mismatch() {
    let client = seed(1);
    let fee_payer = seed(2);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let payload = signed_group(
        &client,
        &fee_payer,
        pay_to,
        1_000_000,
        USDC_TESTNET_ASA_ID,
        2000,
    );
    let req = request_json(&payload, &pay_to, "2", &USDC_TESTNET_ASA_ID.to_string());
    let addrs = vec![fee_payer.address().to_string()];
    let resp = verify_request_json(&MockRpc::default(), &addrs, &req, sign_with(&fee_payer)).await;
    assert_eq!(reason(&resp), "invalid_exact_avm_amount_mismatch");
}

#[tokio::test]
async fn verify_rejects_receiver_mismatch() {
    let client = seed(1);
    let fee_payer = seed(2);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let other = AlgorandAddress::from_public_key([4u8; 32]);
    let payload = signed_group(
        &client,
        &fee_payer,
        pay_to,
        1_000_000,
        USDC_TESTNET_ASA_ID,
        2000,
    );
    let req = request_json(
        &payload,
        &other,
        "1000000",
        &USDC_TESTNET_ASA_ID.to_string(),
    );
    let addrs = vec![fee_payer.address().to_string()];
    let resp = verify_request_json(&MockRpc::default(), &addrs, &req, sign_with(&fee_payer)).await;
    assert_eq!(reason(&resp), "invalid_exact_avm_receiver_mismatch");
}

#[tokio::test]
async fn verify_rejects_asset_mismatch() {
    let client = seed(1);
    let fee_payer = seed(2);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let payload = signed_group(
        &client,
        &fee_payer,
        pay_to,
        1_000_000,
        USDC_TESTNET_ASA_ID,
        2000,
    );
    let req = request_json(&payload, &pay_to, "1000000", "1");
    let addrs = vec![fee_payer.address().to_string()];
    let resp = verify_request_json(&MockRpc::default(), &addrs, &req, sign_with(&fee_payer)).await;
    assert_eq!(reason(&resp), "invalid_exact_avm_asset_mismatch");
}

#[tokio::test]
async fn verify_rejects_wrong_genesis_hash() {
    let client = seed(1);
    let fee_payer = seed(2);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let payload = signed_group_on(
        AlgorandChainReference::MAINNET,
        &client,
        &fee_payer,
        pay_to,
        1_000_000,
        USDC_TESTNET_ASA_ID,
        2000,
    );
    let req = request_json(
        &payload,
        &pay_to,
        "1000000",
        &USDC_TESTNET_ASA_ID.to_string(),
    );
    let addrs = vec![fee_payer.address().to_string()];
    let resp = verify_request_json(&MockRpc::default(), &addrs, &req, sign_with(&fee_payer)).await;
    assert_eq!(reason(&resp), "invalid_exact_avm_network_mismatch");
}

#[tokio::test]
async fn verify_rejects_aggregated_facilitator_fees() {
    let client = seed(1);
    let fee_payer = seed(2);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let chain = AlgorandChainReference::TESTNET;
    let mut pay_a = Transaction::new(TxnType::Pay, fee_payer.address());
    pay_a.receiver = Some(fee_payer.address());
    pay_a.fee = 8_000;
    pay_a.first_valid = 1_000;
    pay_a.last_valid = 2_000;
    pay_a.genesis_id = chain.genesis_id().to_owned();
    pay_a.genesis_hash = chain.genesis_hash();
    let pay_b = pay_a.clone();
    let mut axfer = Transaction::new(TxnType::Axfer, client.address());
    axfer.asset_id = USDC_TESTNET_ASA_ID;
    axfer.asset_amount = 1_000_000;
    axfer.asset_receiver = Some(pay_to);
    axfer.first_valid = 1_000;
    axfer.last_valid = 2_000;
    axfer.genesis_id = chain.genesis_id().to_owned();
    axfer.genesis_hash = chain.genesis_hash();
    let grouped = assign_group(vec![pay_a, pay_b, axfer]);
    let payload = ExactAvmPayload {
        payment_group: vec![
            b64(&encode_signed(&SignedTransaction {
                sig: None,
                txn: grouped[0].clone(),
            })),
            b64(&encode_signed(&SignedTransaction {
                sig: None,
                txn: grouped[1].clone(),
            })),
            b64(&encode_signed(&client.sign(&grouped[2]))),
        ],
        payment_index: 2,
    };
    let req = request_json(
        &payload,
        &pay_to,
        "1000000",
        &USDC_TESTNET_ASA_ID.to_string(),
    );
    let addrs = vec![fee_payer.address().to_string()];
    let resp = verify_request_json(&MockRpc::default(), &addrs, &req, sign_with(&fee_payer)).await;
    assert_eq!(reason(&resp), "invalid_exact_avm_group_size_exceeded");
}

#[tokio::test]
async fn verify_rejects_fee_too_high() {
    let client = seed(1);
    let fee_payer = seed(2);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let payload = signed_group(
        &client,
        &fee_payer,
        pay_to,
        1_000_000,
        USDC_TESTNET_ASA_ID,
        50_000,
    );
    let req = request_json(
        &payload,
        &pay_to,
        "1000000",
        &USDC_TESTNET_ASA_ID.to_string(),
    );
    let addrs = vec![fee_payer.address().to_string()];
    let resp = verify_request_json(&MockRpc::default(), &addrs, &req, sign_with(&fee_payer)).await;
    assert_eq!(reason(&resp), "invalid_exact_avm_fee_too_high");
}

#[tokio::test]
async fn verify_rejects_unsigned_non_facilitator() {
    let client = seed(1);
    let fee_payer = seed(2);
    let stranger = seed(8);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let payload = signed_group(
        &client,
        &stranger,
        pay_to,
        1_000_000,
        USDC_TESTNET_ASA_ID,
        2000,
    );
    let req = request_json(
        &payload,
        &pay_to,
        "1000000",
        &USDC_TESTNET_ASA_ID.to_string(),
    );
    let addrs = vec![fee_payer.address().to_string()];
    let resp = verify_request_json(&MockRpc::default(), &addrs, &req, sign_with(&fee_payer)).await;
    assert_eq!(reason(&resp), "invalid_exact_avm_unsigned_non_facilitator");
}

#[tokio::test]
async fn verify_rejects_simulation_failure() {
    let client = seed(1);
    let fee_payer = seed(2);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let payload = signed_group(
        &client,
        &fee_payer,
        pay_to,
        1_000_000,
        USDC_TESTNET_ASA_ID,
        2000,
    );
    let req = request_json(
        &payload,
        &pay_to,
        "1000000",
        &USDC_TESTNET_ASA_ID.to_string(),
    );
    let mut rpc = MockRpc::default();
    rpc.simulate.failure_message = Some("overspend".to_owned());
    let addrs = vec![fee_payer.address().to_string()];
    let resp = verify_request_json(&rpc, &addrs, &req, sign_with(&fee_payer)).await;
    assert_eq!(reason(&resp), "invalid_exact_avm_simulation_failed");
}

#[tokio::test]
async fn verify_rejects_bad_payment_signature() {
    let client = seed(1);
    let fee_payer = seed(2);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let mut payload = signed_group(
        &client,
        &fee_payer,
        pay_to,
        1_000_000,
        USDC_TESTNET_ASA_ID,
        2000,
    );
    let raw = base64::engine::general_purpose::STANDARD
        .decode(&payload.payment_group[1])
        .unwrap();
    let mut stxn = crate::chain::codec::decode_signed(&raw).unwrap();
    if let Some(sig) = stxn.sig.as_mut() {
        sig[0] ^= 0xff;
    }
    payload.payment_group[1] = b64(&encode_signed(&stxn));
    let req = request_json(
        &payload,
        &pay_to,
        "1000000",
        &USDC_TESTNET_ASA_ID.to_string(),
    );
    let addrs = vec![fee_payer.address().to_string()];
    let resp = verify_request_json(&MockRpc::default(), &addrs, &req, sign_with(&fee_payer)).await;
    assert_eq!(reason(&resp), "invalid_exact_avm_invalid_signature");
}

#[tokio::test]
async fn settle_success_and_duplicate() {
    let client = seed(1);
    let fee_payer = seed(2);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let payload = signed_group(
        &client,
        &fee_payer,
        pay_to,
        1_000_000,
        USDC_TESTNET_ASA_ID,
        2000,
    );
    let req = request_json(
        &payload,
        &pay_to,
        "1000000",
        &USDC_TESTNET_ASA_ID.to_string(),
    );
    let addrs = vec![fee_payer.address().to_string()];
    let cache = SettlementCache::new();
    let rpc = MockRpc::default();
    let first = settle_request(
        &rpc,
        &addrs,
        &cache,
        1,
        &req,
        sign_with(&fee_payer),
        |_txid, _rounds| async { Ok(()) },
    )
    .await;
    assert!(first.is_success(), "{first:?}");
    let second = settle_request(
        &rpc,
        &addrs,
        &cache,
        1,
        &req,
        sign_with(&fee_payer),
        |_txid, _rounds| async { Ok(()) },
    )
    .await;
    match second {
        SettleResponse::Failure { reason, .. } => {
            assert_eq!(reason, ErrorReason::DuplicateSettlement);
        }
        other => panic!("expected duplicate, got {other:?}"),
    }
}

#[cfg(feature = "client")]
#[tokio::test]
async fn client_builds_sponsored_group() {
    let client = seed(1);
    let fee_payer = seed(2);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let payload = crate::exact::client::create_payment_group(
        &client,
        &MockRpc::default(),
        pay_to,
        USDC_TESTNET_ASA_ID,
        1_000_000,
        Some(fee_payer.address()),
        AlgorandChainReference::TESTNET,
    )
    .await
    .unwrap();
    assert_eq!(payload.payment_index, 1);
    assert_eq!(payload.payment_group.len(), 2);
    let pay = crate::chain::codec::decode_signed(
        &base64::engine::general_purpose::STANDARD
            .decode(&payload.payment_group[0])
            .unwrap(),
    )
    .unwrap();
    assert!(pay.sig.is_none());
    assert_eq!(pay.txn.txn_type, TxnType::Pay);
    assert!(pay.txn.fee >= 1000);
    let axfer = crate::chain::codec::decode_signed(
        &base64::engine::general_purpose::STANDARD
            .decode(&payload.payment_group[1])
            .unwrap(),
    )
    .unwrap();
    assert!(axfer.has_signature());
    assert_eq!(axfer.txn.asset_amount, 1_000_000);
    assert_eq!(
        axfer.txn.genesis_hash,
        crate::chain::types::ALGORAND_TESTNET_GENESIS_HASH
    );
    assert_ne!(
        axfer.txn.genesis_hash.as_slice(),
        AlgorandChainReference::TESTNET.as_str().as_bytes()
    );
    assert_eq!(pay.txn.genesis_hash, axfer.txn.genesis_hash);
}

#[test]
fn encode_size_used_for_fee() {
    let txn = Transaction::new(TxnType::Pay, AlgorandAddress::from_public_key([1u8; 32]));
    let size = encode_txn(&txn).len();
    assert!(size > 0);
    assert_eq!(crate::chain::codec::txn_fee(0, 1000, size), 1000);
    assert_eq!(crate::chain::codec::txn_fee(10, 1000, 50), 1000);
}

#[tokio::test]
async fn verify_rejects_payment_aclose() {
    let client = seed(1);
    let fee_payer = seed(2);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let payload = signed_group_mut_axfer(&client, &fee_payer, pay_to, |axfer| {
        axfer.asset_close_to = Some(AlgorandAddress::from_public_key([9u8; 32]));
    });
    let req = request_json(
        &payload,
        &pay_to,
        "1000000",
        &USDC_TESTNET_ASA_ID.to_string(),
    );
    let addrs = vec![fee_payer.address().to_string()];
    let resp = verify_request_json(&MockRpc::default(), &addrs, &req, sign_with(&fee_payer)).await;
    assert_eq!(reason(&resp), "invalid_exact_avm_payload");
}

#[tokio::test]
async fn verify_rejects_payment_asnd() {
    let client = seed(1);
    let fee_payer = seed(2);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let payload = signed_group_mut_axfer(&client, &fee_payer, pay_to, |axfer| {
        axfer.asset_sender = Some(AlgorandAddress::from_public_key([9u8; 32]));
    });
    let req = request_json(
        &payload,
        &pay_to,
        "1000000",
        &USDC_TESTNET_ASA_ID.to_string(),
    );
    let addrs = vec![fee_payer.address().to_string()];
    let resp = verify_request_json(&MockRpc::default(), &addrs, &req, sign_with(&fee_payer)).await;
    assert_eq!(reason(&resp), "invalid_exact_avm_payload");
}

#[tokio::test]
async fn verify_rejects_payment_rekey() {
    let client = seed(1);
    let fee_payer = seed(2);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let payload = signed_group_mut_axfer(&client, &fee_payer, pay_to, |axfer| {
        axfer.rekey_to = Some(AlgorandAddress::from_public_key([9u8; 32]));
    });
    let req = request_json(
        &payload,
        &pay_to,
        "1000000",
        &USDC_TESTNET_ASA_ID.to_string(),
    );
    let addrs = vec![fee_payer.address().to_string()];
    let resp = verify_request_json(&MockRpc::default(), &addrs, &req, sign_with(&fee_payer)).await;
    assert_eq!(reason(&resp), "invalid_exact_avm_payload");
}

#[tokio::test]
async fn verify_rejects_payment_close_remainder() {
    let client = seed(1);
    let fee_payer = seed(2);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let payload = signed_group_mut_axfer(&client, &fee_payer, pay_to, |axfer| {
        axfer.close_remainder_to = Some(AlgorandAddress::from_public_key([9u8; 32]));
    });
    let req = request_json(
        &payload,
        &pay_to,
        "1000000",
        &USDC_TESTNET_ASA_ID.to_string(),
    );
    let addrs = vec![fee_payer.address().to_string()];
    let resp = verify_request_json(&MockRpc::default(), &addrs, &req, sign_with(&fee_payer)).await;
    assert_eq!(reason(&resp), "invalid_exact_avm_payload");
}

#[tokio::test]
async fn verify_rejects_extra_signed_axfer() {
    let client = seed(1);
    let fee_payer = seed(2);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let mut axfer_a = Transaction::new(TxnType::Axfer, client.address());
    axfer_a.asset_id = USDC_TESTNET_ASA_ID;
    axfer_a.asset_amount = 1_000_000;
    axfer_a.asset_receiver = Some(pay_to);
    axfer_a.first_valid = 1_000;
    axfer_a.last_valid = 2_000;
    axfer_a.genesis_id = AlgorandChainReference::TESTNET.genesis_id().to_owned();
    axfer_a.genesis_hash = AlgorandChainReference::TESTNET.genesis_hash();
    let mut axfer_b = axfer_a.clone();
    axfer_b.asset_amount = 1;
    let grouped = assign_group(vec![axfer_a, axfer_b]);
    let payload = ExactAvmPayload {
        payment_group: vec![
            b64(&encode_signed(&client.sign(&grouped[0]))),
            b64(&encode_signed(&client.sign(&grouped[1]))),
        ],
        payment_index: 0,
    };
    let req = request_json(
        &payload,
        &pay_to,
        "1000000",
        &USDC_TESTNET_ASA_ID.to_string(),
    );
    let addrs = vec![fee_payer.address().to_string()];
    let resp = verify_request_json(&MockRpc::default(), &addrs, &req, sign_with(&fee_payer)).await;
    assert_eq!(reason(&resp), "invalid_exact_avm_payload");
}

#[tokio::test]
async fn verify_rejects_group_len_3() {
    let group: Vec<String> = (0..3).map(|_| "AA==".to_owned()).collect();
    let req = request_json(
        &ExactAvmPayload {
            payment_group: group,
            payment_index: 0,
        },
        &AlgorandAddress::ZERO,
        "1",
        "1",
    );
    let resp = verify_request_json(&MockRpc::default(), &[], &req, |_, _| {
        Err(invalid("invalid_exact_avm_invalid_fee_payer"))
    })
    .await;
    assert_eq!(reason(&resp), "invalid_exact_avm_group_size_exceeded");
}

#[tokio::test]
async fn verify_accepts_payment_only() {
    let client = seed(1);
    let fee_payer = seed(2);
    let pay_to = AlgorandAddress::from_public_key([3u8; 32]);
    let payload = signed_payment_only(&client, pay_to);
    let req = request_json(
        &payload,
        &pay_to,
        "1000000",
        &USDC_TESTNET_ASA_ID.to_string(),
    );
    let addrs = vec![fee_payer.address().to_string()];
    let resp = verify_request_json(&MockRpc::default(), &addrs, &req, sign_with(&fee_payer)).await;
    assert_eq!(reason(&resp), "valid");
}
