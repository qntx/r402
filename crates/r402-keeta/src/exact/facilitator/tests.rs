//! Unit tests for Keeta exact verify/settle, matching TS `invalidReason`s.

#![allow(
    clippy::panic,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::clone_on_copy,
    clippy::needless_pass_by_value,
    clippy::excessive_nesting,
    reason = "test assertions with known JSON structure"
)]

use std::sync::Arc;

use keetanetwork_account::{
    Account, Accountable, GenericAccount, KeyED25519, KeyPairType, Keyable,
};
use keetanetwork_block::{
    AccountRef, Amount, Block, BlockBuilder, BlockHash, Hashable, Send, SetRep,
};
use r402_core::wire::{SettleResponse, VerifyResponse};
use serde_json::{Value, json};

use super::{SettlementQueue, settle_request, verify_request_json};
use crate::chain::KeetaChainReference;
use crate::chain::provider::{KeetaAclView, KeetaPreflight, KeetaRpcError};

const AMOUNT: &str = "1000000000";
const EXTERNAL: &str = "ext-ref-abc123";
const NETWORK: &str = "keeta:1413829460";

#[derive(Clone)]
struct MockPreflight {
    balance: Result<Amount, ()>,
    head: Result<Option<BlockHash>, ()>,
    acls: Result<Vec<KeetaAclView>, ()>,
}

impl Default for MockPreflight {
    fn default() -> Self {
        Self {
            balance: Ok(Amount::from(1_000_000_000u64)),
            head: Ok(None),
            acls: Ok(Vec::new()),
        }
    }
}

impl KeetaPreflight for MockPreflight {
    async fn balance(&self, _account: &str, _token: &str) -> Result<Amount, KeetaRpcError> {
        self.balance
            .clone()
            .map_err(|()| KeetaRpcError::Rpc("balance failed".to_owned()))
    }

    async fn head_hash(&self, _account: &str) -> Result<Option<BlockHash>, KeetaRpcError> {
        self.head
            .clone()
            .map_err(|()| KeetaRpcError::Rpc("state failed".to_owned()))
    }

    async fn acls_for_signer(&self, _signer: &str) -> Result<Vec<KeetaAclView>, KeetaRpcError> {
        self.acls
            .clone()
            .map_err(|()| KeetaRpcError::Rpc("acl failed".to_owned()))
    }
}

struct Actors {
    payer: AccountRef,
    pay_to: AccountRef,
    token: AccountRef,
    other: AccountRef,
}

fn ed25519(seed_byte: u8) -> AccountRef {
    let account = Account::<KeyED25519>::try_from(Accountable::KeyAndType(
        Keyable::from([seed_byte; 32]),
        KeyPairType::ED25519,
    ))
    .expect("test account");
    Arc::new(GenericAccount::Ed25519(account))
}

fn actors() -> Actors {
    let payer = ed25519(1);
    let pay_to = ed25519(2);
    let other = ed25519(3);
    let token = Arc::new(
        payer
            .generate_identifier(KeyPairType::TOKEN, None, 0)
            .expect("token"),
    );
    Actors {
        payer,
        pay_to,
        token,
        other,
    }
}

fn signed_send(
    account: &AccountRef,
    signer: &AccountRef,
    token: &AccountRef,
    to: &AccountRef,
    amount: &str,
    external: Option<&str>,
    network: u32,
) -> String {
    let send = Send {
        to: Arc::clone(to),
        amount: amount.parse().expect("amount"),
        token: Arc::clone(token),
        external: external.map(str::to_owned),
    };
    let unsigned = BlockBuilder::default()
        .with_network(network)
        .with_account(Arc::clone(account))
        .with_signer(Arc::clone(signer))
        .as_opening()
        .with_operation(send)
        .build()
        .expect("block must build");
    let block = unsigned.sign().expect("block must sign");
    encode(&block)
}

fn signed_ops(
    account: &AccountRef,
    ops: Vec<keetanetwork_block::Operation>,
    network: u32,
) -> String {
    let mut builder = BlockBuilder::default()
        .with_network(network)
        .with_account(Arc::clone(account))
        .as_opening();
    for op in ops {
        builder = builder.with_operation(op);
    }
    let block = builder.build().expect("build").sign().expect("sign");
    encode(&block)
}

fn encode(block: &Block) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, block.to_bytes())
}

fn send_op(actors: &Actors, amount: &str, external: Option<&str>) -> Send {
    Send {
        to: Arc::clone(&actors.pay_to),
        amount: amount.parse().expect("amount"),
        token: Arc::clone(&actors.token),
        external: external.map(str::to_owned),
    }
}

fn request(actors: &Actors, block: &str, overrides: Value) -> Value {
    let mut req = json!({
        "x402Version": 2,
        "paymentPayload": {
            "x402Version": 2,
            "accepted": {
                "scheme": "exact",
                "network": NETWORK,
                "amount": AMOUNT,
                "asset": actors.token.to_string(),
                "payTo": actors.pay_to.to_string(),
                "maxTimeoutSeconds": 60
            },
            "payload": { "block": block }
        },
        "paymentRequirements": {
            "scheme": "exact",
            "network": NETWORK,
            "amount": AMOUNT,
            "asset": actors.token.to_string(),
            "payTo": actors.pay_to.to_string(),
            "maxTimeoutSeconds": 60
        }
    });
    merge(&mut req, &overrides);
    req
}

fn merge(target: &mut Value, patch: &Value) {
    if let (Some(t), Some(p)) = (target.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            if t.get(k).is_some_and(Value::is_object) && v.is_object() {
                if let Some(child) = t.get_mut(k) {
                    merge(child, v);
                }
            } else {
                t.insert(k.clone(), v.clone());
            }
        }
    }
}

fn reason(resp: &VerifyResponse) -> &str {
    match resp {
        VerifyResponse::Invalid { reason, .. } => reason.as_str(),
        other => panic!("expected invalid, got {other:?}"),
    }
}

fn testnet() -> u32 {
    KeetaChainReference::TESTNET.network_id()
}

#[tokio::test]
async fn valid_payment() {
    let actors = actors();
    let block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let resp = verify_request_json(
        &MockPreflight::default(),
        &[],
        &request(&actors, &block, json!({})),
    )
    .await;
    match resp {
        VerifyResponse::Valid { payer, .. } => {
            assert_eq!(payer, actors.payer.to_string());
        }
        other => panic!("expected valid, got {other:?}"),
    }
}

#[tokio::test]
async fn valid_with_matching_external() {
    let actors = actors();
    let block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        Some(EXTERNAL),
        testnet(),
    );
    let req = request(
        &actors,
        &block,
        json!({ "paymentRequirements": { "extra": { "external": EXTERNAL } } }),
    );
    let resp = verify_request_json(&MockPreflight::default(), &[], &req).await;
    assert!(resp.is_valid());
}

#[tokio::test]
async fn valid_when_block_has_external_but_requirements_do_not() {
    let actors = actors();
    let block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        Some(EXTERNAL),
        testnet(),
    );
    let resp = verify_request_json(
        &MockPreflight::default(),
        &[],
        &request(&actors, &block, json!({})),
    )
    .await;
    assert!(resp.is_valid());
}

#[tokio::test]
async fn rejects_unsupported_version() {
    let actors = actors();
    let block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let req = request(
        &actors,
        &block,
        json!({ "paymentPayload": { "x402Version": 1 } }),
    );
    let resp = verify_request_json(&MockPreflight::default(), &[], &req).await;
    assert_eq!(
        reason(&resp),
        "invalid_exact_keeta_payload_unsupported_version"
    );
}

#[tokio::test]
async fn rejects_unsupported_scheme() {
    let actors = actors();
    let block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let req = request(
        &actors,
        &block,
        json!({ "paymentPayload": { "accepted": { "scheme": "other" } } }),
    );
    let resp = verify_request_json(&MockPreflight::default(), &[], &req).await;
    assert_eq!(reason(&resp), "unsupported_scheme");
}

#[tokio::test]
async fn rejects_malformed_network() {
    let actors = actors();
    let block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let req = request(
        &actors,
        &block,
        json!({ "paymentRequirements": { "network": "keetaX123" } }),
    );
    let resp = verify_request_json(&MockPreflight::default(), &[], &req).await;
    assert_eq!(
        reason(&resp),
        "invalid_exact_keeta_requirements_network_malformed"
    );
}

#[tokio::test]
async fn rejects_non_integer_network_id() {
    let actors = actors();
    let block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let req = request(
        &actors,
        &block,
        json!({ "paymentRequirements": { "network": "keeta:notanumber" } }),
    );
    let resp = verify_request_json(&MockPreflight::default(), &[], &req).await;
    assert_eq!(reason(&resp), "invalid_exact_keeta_requirements_network_id");
}

#[tokio::test]
async fn rejects_accepted_network_mismatch() {
    let actors = actors();
    let block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let req = request(
        &actors,
        &block,
        json!({ "paymentPayload": { "accepted": { "network": "keeta:21378" } } }),
    );
    let resp = verify_request_json(&MockPreflight::default(), &[], &req).await;
    assert_eq!(reason(&resp), "network_mismatch");
}

#[tokio::test]
async fn rejects_undecodable_block() {
    let actors = actors();
    let req = request(&actors, "not-valid-base64-der!!!", json!({}));
    let resp = verify_request_json(&MockPreflight::default(), &[], &req).await;
    assert_eq!(
        reason(&resp),
        "invalid_exact_keeta_payload_block_could_not_be_decoded"
    );
}

#[tokio::test]
async fn rejects_tampered_signature() {
    let actors = actors();
    let mut block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let mut bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &block).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    block = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
    let resp = verify_request_json(
        &MockPreflight::default(),
        &[],
        &request(&actors, &block, json!({})),
    )
    .await;
    assert_eq!(
        reason(&resp),
        "invalid_exact_keeta_payload_block_could_not_be_decoded"
    );
}

#[tokio::test]
async fn rejects_block_network_mismatch() {
    let actors = actors();
    let block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        KeetaChainReference::MAINNET.network_id(),
    );
    let resp = verify_request_json(
        &MockPreflight::default(),
        &[],
        &request(&actors, &block, json!({})),
    )
    .await;
    assert_eq!(reason(&resp), "network_mismatch");
}

#[tokio::test]
async fn rejects_two_operations() {
    let actors = actors();
    let block = signed_ops(
        &actors.payer,
        vec![
            send_op(&actors, AMOUNT, None).into(),
            send_op(&actors, AMOUNT, None).into(),
        ],
        testnet(),
    );
    let resp = verify_request_json(
        &MockPreflight::default(),
        &[],
        &request(&actors, &block, json!({})),
    )
    .await;
    assert_eq!(
        reason(&resp),
        "invalid_exact_keeta_payload_operations_length"
    );
}

#[tokio::test]
async fn rejects_non_send() {
    let actors = actors();
    let block = signed_ops(
        &actors.payer,
        vec![
            SetRep {
                to: Arc::clone(&actors.pay_to),
            }
            .into(),
        ],
        testnet(),
    );
    let resp = verify_request_json(
        &MockPreflight::default(),
        &[],
        &request(&actors, &block, json!({})),
    )
    .await;
    assert_eq!(
        reason(&resp),
        "invalid_exact_keeta_payload_payment_operation_type"
    );
}

#[tokio::test]
async fn rejects_asset_mismatch() {
    let actors = actors();
    let block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let req = request(
        &actors,
        &block,
        json!({ "paymentRequirements": { "asset": actors.other.to_string() } }),
    );
    let resp = verify_request_json(&MockPreflight::default(), &[], &req).await;
    assert_eq!(
        reason(&resp),
        "invalid_exact_keeta_payload_payment_asset_mismatch"
    );
}

#[tokio::test]
async fn rejects_amount_mismatch() {
    let actors = actors();
    let block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let req = request(
        &actors,
        &block,
        json!({ "paymentRequirements": { "amount": "999999" } }),
    );
    let resp = verify_request_json(&MockPreflight::default(), &[], &req).await;
    assert_eq!(
        reason(&resp),
        "invalid_exact_keeta_payload_payment_amount_mismatch"
    );
}

#[tokio::test]
async fn rejects_invalid_amount() {
    let actors = actors();
    let block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let req = request(
        &actors,
        &block,
        json!({ "paymentRequirements": { "amount": "not-a-number" } }),
    );
    let resp = verify_request_json(&MockPreflight::default(), &[], &req).await;
    assert_eq!(
        reason(&resp),
        "invalid_exact_keeta_payload_payment_amount_invalid"
    );
}

#[tokio::test]
async fn rejects_pay_to_mismatch() {
    let actors = actors();
    let block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let req = request(
        &actors,
        &block,
        json!({ "paymentRequirements": { "payTo": actors.payer.to_string() } }),
    );
    let resp = verify_request_json(&MockPreflight::default(), &[], &req).await;
    assert_eq!(
        reason(&resp),
        "invalid_exact_keeta_payload_payment_to_mismatch"
    );
}

#[tokio::test]
async fn rejects_external_mismatch() {
    let actors = actors();
    let block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        Some(EXTERNAL),
        testnet(),
    );
    let req = request(
        &actors,
        &block,
        json!({ "paymentRequirements": { "extra": { "external": "other" } } }),
    );
    let resp = verify_request_json(&MockPreflight::default(), &[], &req).await;
    assert_eq!(
        reason(&resp),
        "invalid_exact_keeta_payload_payment_external_mismatch"
    );
}

#[tokio::test]
async fn rejects_payer_is_facilitator() {
    let actors = actors();
    let block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let resp = verify_request_json(
        &MockPreflight::default(),
        &[actors.payer.to_string()],
        &request(&actors, &block, json!({})),
    )
    .await;
    assert_eq!(
        reason(&resp),
        "invalid_exact_keeta_payload_payer_is_facilitator"
    );
}

#[tokio::test]
async fn rejects_insufficient_funds() {
    let actors = actors();
    let block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let preflight = MockPreflight {
        balance: Ok(Amount::from(1u64)),
        ..MockPreflight::default()
    };
    let resp = verify_request_json(&preflight, &[], &request(&actors, &block, json!({}))).await;
    assert_eq!(
        reason(&resp),
        "invalid_exact_keeta_payload_insufficient_funds"
    );
}

#[tokio::test]
async fn rejects_previous_head_mismatch() {
    let actors = actors();
    let block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let preflight = MockPreflight {
        head: Ok(Some(BlockHash::from([0u8; 32]))),
        ..MockPreflight::default()
    };
    let resp = verify_request_json(&preflight, &[], &request(&actors, &block, json!({}))).await;
    assert_eq!(
        reason(&resp),
        "invalid_exact_keeta_payload_previous_head_mismatch"
    );
}

#[tokio::test]
async fn rejects_missing_permission() {
    let actors = actors();
    let block = signed_send(
        &actors.other,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let resp = verify_request_json(
        &MockPreflight::default(),
        &[],
        &request(&actors, &block, json!({})),
    )
    .await;
    assert_eq!(
        reason(&resp),
        "invalid_exact_keeta_payload_missing_permission"
    );
}

#[tokio::test]
async fn accepts_delegated_send_with_permission() {
    let actors = actors();
    let block = signed_send(
        &actors.other,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let preflight = MockPreflight {
        acls: Ok(vec![KeetaAclView {
            entity: actors.other.to_string(),
            owner: false,
            send_on_behalf: true,
        }]),
        ..MockPreflight::default()
    };
    let resp = verify_request_json(&preflight, &[], &request(&actors, &block, json!({}))).await;
    assert!(resp.is_valid(), "{resp:?}");
}

#[tokio::test]
async fn generated_der_round_trips() {
    let actors = actors();
    let encoded = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &encoded).unwrap();
    let block = Block::try_from(bytes.as_slice()).unwrap();
    assert_eq!(block.to_bytes(), bytes.as_slice());
}

#[tokio::test]
async fn settle_success() {
    let actors = actors();
    let block = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let queue = SettlementQueue::mock(vec!["fee-payer".to_owned()], |_addr, signed| {
        Box::pin(async move { Ok(signed.hash().to_string()) })
    });
    let req = request(&actors, &block, json!({}));
    let first = settle_request(&MockPreflight::default(), &[], &queue, &req).await;
    match first {
        SettleResponse::Success {
            transaction, payer, ..
        } => {
            assert!(!transaction.is_empty());
            assert_eq!(payer, actors.payer.to_string());
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn settle_duplicate_in_flight() {
    let actors = actors();
    let encoded = signed_send(
        &actors.payer,
        &actors.payer,
        &actors.token,
        &actors.pay_to,
        AMOUNT,
        None,
        testnet(),
    );
    let decoded = super::decode_block(&encoded).unwrap();
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let started_cb = Arc::clone(&started);
    let release_cb = Arc::clone(&release);
    let queue = Arc::new(SettlementQueue::mock(
        vec!["fee-payer".to_owned()],
        move |_addr, block| hold_until(Arc::clone(&started_cb), Arc::clone(&release_cb), block),
    ));
    let queue_first = Arc::clone(&queue);
    let first_block = decoded.clone();
    let first = tokio::spawn(async move { queue_first.enqueue("fee-payer", first_block).await });
    started.notified().await;
    let second = queue.enqueue("fee-payer", decoded).await;
    assert!(matches!(second, Err(super::QueueError::Duplicate)));
    release.notify_one();
    assert!(first.await.unwrap().is_ok());
}

fn hold_until(
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    block: Block,
) -> r402_core::facilitator::BoxFuture<'static, Result<String, String>> {
    Box::pin(async move {
        started.notify_one();
        release.notified().await;
        Ok(block.hash().to_string())
    })
}

#[test]
fn spec_payload_json_has_block() {
    let raw = include_str!("../fixtures/ts_payment_payload.json");
    let v: Value = serde_json::from_str(raw).unwrap();
    assert_eq!(v["accepted"]["network"], "keeta:1413829460");
    assert!(v["payload"]["block"].as_str().unwrap().len() > 16);
}
