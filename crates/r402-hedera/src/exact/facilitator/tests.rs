//! Unit tests for Hedera exact verify/settle, matching @x402/hedera reasons.

#![allow(
    clippy::panic,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unused_async,
    unknown_lints,
    clippy::unused_async_trait_impl,
    clippy::field_reassign_with_default,
    clippy::needless_pass_by_value,
    reason = "test assertions with known JSON structure"
)]

use std::str::FromStr;
use std::sync::{Arc, Mutex};

use base64::Engine;
use hedera::{
    AccountId, Hbar, PrivateKey, TokenId, TopicCreateTransaction, TransactionId,
    TransferTransaction,
};
use r402_core::cache::SettlementCache;
use r402_core::chain::{ChainId, ChainProvider};
use r402_core::error::ErrorReason;
use r402_core::wire::{SettleResponse, VerifyResponse};
use serde_json::{Value, json};

use super::{AliasPolicy, HederaFacilitatorOps, settle_request, verify_request_json};
use crate::chain::rpc::{HederaAccountResolution, HederaPreflightResult, HederaSignatureResult};
use crate::chain::{HEDERA_MAINNET_USDC, HEDERA_TESTNET_USDC, HederaProviderError};

const FEE_PAYER: &str = "0.0.5001";
const PAYER: &str = "0.0.9001";
const PAY_TO: &str = "0.0.7001";
const ASSET: &str = "0.0.6001";
const AMOUNT: &str = "1000";

#[derive(Clone)]
struct MockProvider {
    fee_payers: Vec<String>,
    resolve: HederaAccountResolution,
    signature: HederaSignatureResult,
    preflight: HederaPreflightResult,
    signature_err: Option<String>,
    preflight_err: Option<String>,
    submit: Result<String, String>,
    signature_calls: Arc<Mutex<Vec<String>>>,
    preflight_calls: Arc<Mutex<Vec<(String, String)>>>,
}

impl Default for MockProvider {
    fn default() -> Self {
        Self {
            fee_payers: vec![FEE_PAYER.to_owned()],
            resolve: HederaAccountResolution {
                exists: true,
                is_alias: false,
            },
            signature: HederaSignatureResult::ok(),
            preflight: HederaPreflightResult::ok(),
            signature_err: None,
            preflight_err: None,
            submit: Ok("0.0.5001@1700000001.000000000".to_owned()),
            signature_calls: Arc::new(Mutex::new(Vec::new())),
            preflight_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl ChainProvider for MockProvider {
    fn signer_addresses(&self) -> Vec<String> {
        self.fee_payers.clone()
    }

    fn chain_id(&self) -> ChainId {
        ChainId::new("hedera", "testnet")
    }
}

impl HederaFacilitatorOps for MockProvider {
    fn fee_payer_ids(&self) -> Vec<String> {
        self.fee_payers.clone()
    }

    async fn verify_payer_signature(
        &self,
        payer: &str,
        _transaction_base64: &str,
        _network: &str,
    ) -> Result<HederaSignatureResult, HederaProviderError> {
        self.signature_calls.lock().unwrap().push(payer.to_owned());
        if let Some(err) = &self.signature_err {
            return Err(HederaProviderError::Parse(err.clone()));
        }
        Ok(self.signature.clone())
    }

    async fn preflight_transfer(
        &self,
        payer: &str,
        _pay_to: &str,
        _asset: &str,
        amount: &str,
        _network: &str,
    ) -> Result<HederaPreflightResult, HederaProviderError> {
        self.preflight_calls
            .lock()
            .unwrap()
            .push((payer.to_owned(), amount.to_owned()));
        if let Some(err) = &self.preflight_err {
            return Err(HederaProviderError::Parse(err.clone()));
        }
        Ok(self.preflight.clone())
    }

    async fn resolve_account(
        &self,
        _account_id_or_alias: &str,
        _network: &str,
    ) -> Result<HederaAccountResolution, HederaProviderError> {
        Ok(self.resolve)
    }

    async fn sign_and_submit(
        &self,
        _transaction_base64: &str,
        _fee_payer: &str,
        _network: &str,
    ) -> Result<String, HederaProviderError> {
        self.submit.clone().map_err(HederaProviderError::Submit)
    }
}

fn transfer_b64(fee_payer: &str, payer: &str, pay_to: &str, asset: &str, amount: &str) -> String {
    let amount: i64 = amount.parse().unwrap();
    let mut tx = TransferTransaction::new();
    if asset == "0.0.0" {
        let _ = tx
            .hbar_transfer(
                AccountId::from_str(payer).unwrap(),
                Hbar::from_tinybars(-amount),
            )
            .hbar_transfer(
                AccountId::from_str(pay_to).unwrap(),
                Hbar::from_tinybars(amount),
            );
    } else {
        let token = TokenId::from_str(asset).unwrap();
        let _ = tx
            .token_transfer(token, AccountId::from_str(payer).unwrap(), -amount)
            .token_transfer(token, AccountId::from_str(pay_to).unwrap(), amount);
    }
    let _ = tx
        .transaction_id(TransactionId::generate(
            AccountId::from_str(fee_payer).unwrap(),
        ))
        .node_account_ids([AccountId::new(0, 0, 3)]);
    tx.freeze().unwrap();
    Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        tx.to_bytes().unwrap(),
    )
}

fn requirements(overrides: Value) -> Value {
    let mut base = json!({
        "scheme": "exact",
        "network": "hedera:testnet",
        "asset": ASSET,
        "amount": AMOUNT,
        "payTo": PAY_TO,
        "maxTimeoutSeconds": 180,
        "extra": { "feePayer": FEE_PAYER },
    });
    if let Some(obj) = overrides.as_object() {
        for (k, v) in obj {
            if k == "extra" {
                base["extra"] = v.clone();
            } else {
                base[k] = v.clone();
            }
        }
    }
    base
}

fn request(requirements: &Value, transaction: &str) -> Value {
    json!({
        "x402Version": 2,
        "paymentPayload": {
            "x402Version": 2,
            "accepted": requirements,
            "payload": { "transaction": transaction },
            "resource": {
                "url": "https://example.com",
                "description": "resource",
                "mimeType": "application/json"
            }
        },
        "paymentRequirements": requirements,
    })
}

fn reason(resp: &VerifyResponse) -> String {
    match resp {
        VerifyResponse::Invalid { reason, .. } => reason.as_str().to_owned(),
        other => panic!("expected invalid, got {other:?}"),
    }
}

async fn verify(provider: &MockProvider, req: &Value) -> VerifyResponse {
    verify_request_json(provider, AliasPolicy::Reject, req).await
}

#[tokio::test]
async fn verifies_valid_payload() {
    let provider = MockProvider::default();
    let reqs = requirements(json!({}));
    let req = request(
        &reqs,
        &transfer_b64(FEE_PAYER, PAYER, PAY_TO, ASSET, AMOUNT),
    );
    let result = verify(&provider, &req).await;
    match result {
        VerifyResponse::Valid { payer, .. } => assert_eq!(payer, PAYER),
        other => panic!("expected valid, got {other:?}"),
    }
}

#[tokio::test]
async fn rejects_unsupported_token() {
    let provider = MockProvider::default();
    let reqs = requirements(json!({}));
    let req = request(
        &reqs,
        &transfer_b64(FEE_PAYER, PAYER, PAY_TO, "0.0.1234", AMOUNT),
    );
    let result = verify(&provider, &req).await;
    assert_eq!(
        reason(&result),
        "invalid_exact_hedera_payload_asset_mismatch"
    );
}

#[tokio::test]
async fn enforces_alias_rejection_by_default() {
    let mut provider = MockProvider::default();
    provider.resolve = HederaAccountResolution {
        exists: false,
        is_alias: true,
    };
    let reqs = requirements(json!({}));
    let req = request(
        &reqs,
        &transfer_b64(FEE_PAYER, PAYER, PAY_TO, ASSET, AMOUNT),
    );
    let result = verify(&provider, &req).await;
    assert_eq!(
        reason(&result),
        "invalid_exact_hedera_payload_pay_to_alias_not_allowed"
    );
}

#[tokio::test]
async fn can_allow_aliases() {
    let mut provider = MockProvider::default();
    provider.resolve = HederaAccountResolution {
        exists: false,
        is_alias: true,
    };
    let alias = AccountId::from(PrivateKey::generate_ed25519().public_key()).to_string();
    let reqs = requirements(json!({ "payTo": alias }));
    let req = request(
        &reqs,
        &transfer_b64(FEE_PAYER, PAYER, &alias, ASSET, AMOUNT),
    );
    let result = verify_request_json(&provider, AliasPolicy::Allow, &req).await;
    assert!(
        result.is_valid(),
        "expected valid alias payment, got {result:?}"
    );
}

#[tokio::test]
async fn rejects_undecodable_transaction() {
    let provider = MockProvider::default();
    let reqs = requirements(json!({}));
    let req = request(&reqs, "not-a-valid-hedera-transaction");
    let result = verify(&provider, &req).await;
    assert_eq!(
        reason(&result),
        "invalid_exact_hedera_payload_transaction_could_not_be_decoded"
    );
}

#[tokio::test]
async fn settles_when_verify_passes() {
    let provider = MockProvider::default();
    let reqs = requirements(json!({}));
    let req = request(
        &reqs,
        &transfer_b64(FEE_PAYER, PAYER, PAY_TO, ASSET, AMOUNT),
    );
    let settled = settle_request(
        &provider,
        AliasPolicy::Reject,
        &SettlementCache::new(),
        &req,
    )
    .await;
    match settled {
        SettleResponse::Success { transaction, .. } => {
            assert!(transaction.contains("0.0.5001@"));
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn rejects_unsupported_scheme() {
    let provider = MockProvider::default();
    let reqs = requirements(json!({ "scheme": "something-else" }));
    let req = request(&reqs, "");
    let result = verify(&provider, &req).await;
    assert_eq!(reason(&result), "unsupported_scheme");
}

#[tokio::test]
async fn rejects_accepted_mismatch() {
    let provider = MockProvider::default();
    let reqs = requirements(json!({}));
    let mut req = request(&reqs, "");
    req["paymentPayload"]["accepted"]["amount"] = json!("999");
    let result = verify(&provider, &req).await;
    assert_eq!(reason(&result), "accepted_payment_requirements_mismatch");
}

#[tokio::test]
async fn rejects_network_mismatch() {
    let provider = MockProvider::default();
    let reqs = requirements(json!({}));
    let mut req = request(&reqs, "");
    req["paymentPayload"]["accepted"]["network"] = json!("hedera:mainnet");
    let result = verify(&provider, &req).await;
    assert_eq!(reason(&result), "network_mismatch");
}

#[tokio::test]
async fn rejects_unsupported_network_value() {
    let provider = MockProvider::default();
    let reqs = requirements(json!({ "network": "eip155:1" }));
    let req = request(&reqs, "");
    let result = verify(&provider, &req).await;
    assert_eq!(reason(&result), "network_mismatch");
}

#[tokio::test]
async fn rejects_invalid_asset() {
    let provider = MockProvider::default();
    let reqs = requirements(json!({ "asset": "invalid-asset" }));
    let req = request(&reqs, "");
    let result = verify(&provider, &req).await;
    assert_eq!(reason(&result), "invalid_asset");
}

#[tokio::test]
async fn rejects_invalid_amount() {
    let provider = MockProvider::default();
    let reqs = requirements(json!({ "amount": "1.23" }));
    let req = request(&reqs, "");
    let result = verify(&provider, &req).await;
    assert_eq!(reason(&result), "invalid_amount");
}

#[tokio::test]
async fn rejects_missing_fee_payer() {
    let provider = MockProvider::default();
    let reqs = requirements(json!({ "extra": {} }));
    let req = request(&reqs, "");
    let result = verify(&provider, &req).await;
    assert_eq!(
        reason(&result),
        "invalid_exact_hedera_payload_missing_fee_payer"
    );
}

#[tokio::test]
async fn rejects_unmanaged_fee_payer() {
    let mut provider = MockProvider::default();
    provider.fee_payers = vec!["0.0.9999".to_owned()];
    let reqs = requirements(json!({}));
    let req = request(&reqs, "");
    let result = verify(&provider, &req).await;
    assert_eq!(reason(&result), "fee_payer_not_managed_by_facilitator");
}

#[tokio::test]
async fn rejects_transaction_fee_payer_mismatch() {
    let provider = MockProvider::default();
    let reqs = requirements(json!({}));
    let req = request(
        &reqs,
        &transfer_b64("0.0.5002", PAYER, PAY_TO, ASSET, AMOUNT),
    );
    let result = verify(&provider, &req).await;
    assert_eq!(
        reason(&result),
        "invalid_exact_hedera_payload_fee_payer_mismatch"
    );
}

#[tokio::test]
async fn rejects_non_transfer() {
    let provider = MockProvider::default();
    let mut tx = TopicCreateTransaction::new();
    let _ = tx
        .transaction_id(TransactionId::generate(
            AccountId::from_str(FEE_PAYER).unwrap(),
        ))
        .node_account_ids([AccountId::new(0, 0, 3)]);
    let key = PrivateKey::generate_ed25519();
    let _ = tx.submit_key(key.public_key());
    tx.freeze().unwrap();
    let b64 = Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        tx.to_bytes().unwrap(),
    );
    let reqs = requirements(json!({}));
    let result = verify(&provider, &request(&reqs, &b64)).await;
    assert_eq!(
        reason(&result),
        "invalid_exact_hedera_payload_contains_non_transfer_ops"
    );
}

#[tokio::test]
async fn rejects_non_zero_hbar_sum() {
    let provider = MockProvider::default();
    let mut tx = TransferTransaction::new();
    let _ = tx
        .hbar_transfer(
            AccountId::from_str(PAYER).unwrap(),
            Hbar::from_tinybars(-1000),
        )
        .hbar_transfer(
            AccountId::from_str(PAY_TO).unwrap(),
            Hbar::from_tinybars(900),
        )
        .token_transfer(
            TokenId::from_str(ASSET).unwrap(),
            AccountId::from_str(PAYER).unwrap(),
            -1000,
        )
        .token_transfer(
            TokenId::from_str(ASSET).unwrap(),
            AccountId::from_str(PAY_TO).unwrap(),
            1000,
        );
    let _ = tx
        .transaction_id(TransactionId::generate(
            AccountId::from_str(FEE_PAYER).unwrap(),
        ))
        .node_account_ids([AccountId::new(0, 0, 3)]);
    tx.freeze().unwrap();
    let b64 = Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        tx.to_bytes().unwrap(),
    );
    let reqs = requirements(json!({}));
    let result = verify(&provider, &request(&reqs, &b64)).await;
    assert_eq!(
        reason(&result),
        "invalid_exact_hedera_payload_hbar_sum_non_zero"
    );
}

#[tokio::test]
async fn rejects_token_payload_with_hbar() {
    let provider = MockProvider::default();
    let mut tx = TransferTransaction::new();
    let _ = tx
        .hbar_transfer(
            AccountId::from_str(PAYER).unwrap(),
            Hbar::from_tinybars(-10),
        )
        .hbar_transfer(
            AccountId::from_str(PAY_TO).unwrap(),
            Hbar::from_tinybars(10),
        )
        .token_transfer(
            TokenId::from_str(ASSET).unwrap(),
            AccountId::from_str(PAYER).unwrap(),
            -1000,
        )
        .token_transfer(
            TokenId::from_str(ASSET).unwrap(),
            AccountId::from_str(PAY_TO).unwrap(),
            1000,
        );
    let _ = tx
        .transaction_id(TransactionId::generate(
            AccountId::from_str(FEE_PAYER).unwrap(),
        ))
        .node_account_ids([AccountId::new(0, 0, 3)]);
    tx.freeze().unwrap();
    let b64 = Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        tx.to_bytes().unwrap(),
    );
    let reqs = requirements(json!({}));
    let result = verify(&provider, &request(&reqs, &b64)).await;
    assert_eq!(
        reason(&result),
        "invalid_exact_hedera_payload_unexpected_hbar_transfers"
    );
}

#[tokio::test]
async fn rejects_fee_payer_sending_hbar() {
    let provider = MockProvider::default();
    let mut tx = TransferTransaction::new();
    let _ = tx
        .hbar_transfer(
            AccountId::from_str(FEE_PAYER).unwrap(),
            Hbar::from_tinybars(-10),
        )
        .hbar_transfer(AccountId::from_str(PAYER).unwrap(), Hbar::from_tinybars(10))
        .token_transfer(
            TokenId::from_str(ASSET).unwrap(),
            AccountId::from_str(PAYER).unwrap(),
            -1000,
        )
        .token_transfer(
            TokenId::from_str(ASSET).unwrap(),
            AccountId::from_str(PAY_TO).unwrap(),
            1000,
        );
    let _ = tx
        .transaction_id(TransactionId::generate(
            AccountId::from_str(FEE_PAYER).unwrap(),
        ))
        .node_account_ids([AccountId::new(0, 0, 3)]);
    tx.freeze().unwrap();
    let b64 = Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        tx.to_bytes().unwrap(),
    );
    let reqs = requirements(json!({}));
    let result = verify(&provider, &request(&reqs, &b64)).await;
    assert_eq!(
        reason(&result),
        "invalid_exact_hedera_payload_fee_payer_transferring_hbar"
    );
}

#[tokio::test]
async fn rejects_non_zero_asset_sum() {
    let provider = MockProvider::default();
    let mut tx = TransferTransaction::new();
    let _ = tx
        .token_transfer(
            TokenId::from_str(ASSET).unwrap(),
            AccountId::from_str(PAYER).unwrap(),
            -1000,
        )
        .token_transfer(
            TokenId::from_str(ASSET).unwrap(),
            AccountId::from_str(PAY_TO).unwrap(),
            900,
        );
    let _ = tx
        .transaction_id(TransactionId::generate(
            AccountId::from_str(FEE_PAYER).unwrap(),
        ))
        .node_account_ids([AccountId::new(0, 0, 3)]);
    tx.freeze().unwrap();
    let b64 = Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        tx.to_bytes().unwrap(),
    );
    let reqs = requirements(json!({}));
    let result = verify(&provider, &request(&reqs, &b64)).await;
    assert_eq!(
        reason(&result),
        "invalid_exact_hedera_payload_asset_sum_non_zero"
    );
}

#[tokio::test]
async fn rejects_fee_payer_sending_asset() {
    let provider = MockProvider::default();
    let mut tx = TransferTransaction::new();
    let _ = tx
        .token_transfer(
            TokenId::from_str(ASSET).unwrap(),
            AccountId::from_str(FEE_PAYER).unwrap(),
            -1,
        )
        .token_transfer(
            TokenId::from_str(ASSET).unwrap(),
            AccountId::from_str(PAYER).unwrap(),
            -999,
        )
        .token_transfer(
            TokenId::from_str(ASSET).unwrap(),
            AccountId::from_str(PAY_TO).unwrap(),
            1000,
        );
    let _ = tx
        .transaction_id(TransactionId::generate(
            AccountId::from_str(FEE_PAYER).unwrap(),
        ))
        .node_account_ids([AccountId::new(0, 0, 3)]);
    tx.freeze().unwrap();
    let b64 = Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        tx.to_bytes().unwrap(),
    );
    let reqs = requirements(json!({}));
    let result = verify(&provider, &request(&reqs, &b64)).await;
    assert_eq!(
        reason(&result),
        "invalid_exact_hedera_payload_fee_payer_transferring_funds"
    );
}

#[tokio::test]
async fn rejects_amount_mismatch() {
    let provider = MockProvider::default();
    let mut tx = TransferTransaction::new();
    let _ = tx
        .token_transfer(
            TokenId::from_str(ASSET).unwrap(),
            AccountId::from_str(PAYER).unwrap(),
            -1000,
        )
        .token_transfer(
            TokenId::from_str(ASSET).unwrap(),
            AccountId::from_str(PAY_TO).unwrap(),
            999,
        )
        .token_transfer(
            TokenId::from_str(ASSET).unwrap(),
            AccountId::from_str("0.0.7002").unwrap(),
            1,
        );
    let _ = tx
        .transaction_id(TransactionId::generate(
            AccountId::from_str(FEE_PAYER).unwrap(),
        ))
        .node_account_ids([AccountId::new(0, 0, 3)]);
    tx.freeze().unwrap();
    let b64 = Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        tx.to_bytes().unwrap(),
    );
    let reqs = requirements(json!({}));
    let result = verify(&provider, &request(&reqs, &b64)).await;
    assert_eq!(
        reason(&result),
        "invalid_exact_hedera_payload_amount_mismatch"
    );
}

#[tokio::test]
async fn rejects_extra_positive_recipients() {
    let provider = MockProvider::default();
    let mut tx = TransferTransaction::new();
    let _ = tx
        .token_transfer(
            TokenId::from_str(ASSET).unwrap(),
            AccountId::from_str(PAYER).unwrap(),
            -1001,
        )
        .token_transfer(
            TokenId::from_str(ASSET).unwrap(),
            AccountId::from_str(PAY_TO).unwrap(),
            1000,
        )
        .token_transfer(
            TokenId::from_str(ASSET).unwrap(),
            AccountId::from_str("0.0.7002").unwrap(),
            1,
        );
    let _ = tx
        .transaction_id(TransactionId::generate(
            AccountId::from_str(FEE_PAYER).unwrap(),
        ))
        .node_account_ids([AccountId::new(0, 0, 3)]);
    tx.freeze().unwrap();
    let b64 = Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        tx.to_bytes().unwrap(),
    );
    let reqs = requirements(json!({}));
    let result = verify(&provider, &request(&reqs, &b64)).await;
    assert_eq!(
        reason(&result),
        "invalid_exact_hedera_payload_extra_positive_transfers"
    );
}

#[tokio::test]
async fn rejects_invalid_pay_to_format() {
    let provider = MockProvider::default();
    let reqs = requirements(json!({ "payTo": "not-an-account" }));
    let req = request(&reqs, "");
    let result = verify(&provider, &req).await;
    assert_eq!(reason(&result), "invalid_exact_hedera_payload_pay_to");
}

#[tokio::test]
async fn settle_fails_when_verify_fails() {
    let provider = MockProvider::default();
    let reqs = requirements(json!({ "amount": "bad" }));
    let settled = settle_request(
        &provider,
        AliasPolicy::Reject,
        &SettlementCache::new(),
        &request(&reqs, ""),
    )
    .await;
    match settled {
        SettleResponse::Failure { reason, .. } => {
            assert_eq!(reason.as_str(), "invalid_amount");
        }
        other => panic!("expected failure, got {other:?}"),
    }
}

#[tokio::test]
async fn settle_transaction_failed_on_submit_error() {
    let mut provider = MockProvider::default();
    provider.submit = Err("submit failed".to_owned());
    let reqs = requirements(json!({}));
    let req = request(
        &reqs,
        &transfer_b64(FEE_PAYER, PAYER, PAY_TO, ASSET, AMOUNT),
    );
    let settled = settle_request(
        &provider,
        AliasPolicy::Reject,
        &SettlementCache::new(),
        &req,
    )
    .await;
    match settled {
        SettleResponse::Failure {
            reason, message, ..
        } => {
            assert_eq!(reason.as_str(), "transaction_failed");
            assert!(message.unwrap().contains("submit failed"));
        }
        other => panic!("expected failure, got {other:?}"),
    }
}

#[tokio::test]
async fn preflight_failure() {
    let mut provider = MockProvider::default();
    provider.preflight =
        HederaPreflightResult::fail("insufficient_balance", "payer has 500, needs 1000");
    let reqs = requirements(json!({}));
    let req = request(
        &reqs,
        &transfer_b64(FEE_PAYER, PAYER, PAY_TO, ASSET, AMOUNT),
    );
    let result = verify(&provider, &req).await;
    assert_eq!(
        reason(&result),
        "invalid_exact_hedera_payload_preflight_failed"
    );
    match result {
        VerifyResponse::Invalid { message, payer, .. } => {
            assert!(message.unwrap().contains("insufficient_balance"));
            assert_eq!(payer.unwrap(), PAYER);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn signature_failure_skips_preflight() {
    let mut provider = MockProvider::default();
    provider.signature = HederaSignatureResult::fail("signature_invalid", "did not sign");
    let reqs = requirements(json!({}));
    let req = request(
        &reqs,
        &transfer_b64(FEE_PAYER, PAYER, PAY_TO, ASSET, AMOUNT),
    );
    let _ = verify(&provider, &req).await;
    assert!(provider.preflight_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn multi_sender_preflight_amounts() {
    let provider = MockProvider::default();
    let mut tx = TransferTransaction::new();
    let token = TokenId::from_str(ASSET).unwrap();
    let _ = tx
        .token_transfer(token, AccountId::from_str("0.0.9001").unwrap(), -600)
        .token_transfer(token, AccountId::from_str("0.0.9002").unwrap(), -400)
        .token_transfer(token, AccountId::from_str(PAY_TO).unwrap(), 1000);
    let _ = tx
        .transaction_id(TransactionId::generate(
            AccountId::from_str(FEE_PAYER).unwrap(),
        ))
        .node_account_ids([AccountId::new(0, 0, 3)]);
    tx.freeze().unwrap();
    let b64 = Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        tx.to_bytes().unwrap(),
    );
    let reqs = requirements(json!({}));
    let result = verify(&provider, &request(&reqs, &b64)).await;
    assert!(result.is_valid());
    let sigs = provider.signature_calls.lock().unwrap().clone();
    assert_eq!(sigs.len(), 2);
    let pre = provider.preflight_calls.lock().unwrap().clone();
    assert!(pre.contains(&("0.0.9001".to_owned(), "600".to_owned())));
    assert!(pre.contains(&("0.0.9002".to_owned(), "400".to_owned())));
}

#[tokio::test]
async fn usdc_mainnet_and_testnet() {
    for (network, asset) in [
        ("hedera:testnet", HEDERA_TESTNET_USDC),
        ("hedera:mainnet", HEDERA_MAINNET_USDC),
    ] {
        let provider = MockProvider::default();
        let reqs = requirements(json!({
            "network": network,
            "asset": asset,
            "amount": "10000",
        }));
        let req = request(
            &reqs,
            &transfer_b64(FEE_PAYER, PAYER, PAY_TO, asset, "10000"),
        );
        let result = verify(&provider, &req).await;
        assert!(result.is_valid(), "{network} {result:?}");
    }
}

#[tokio::test]
async fn duplicate_settle_rejected() {
    let provider = MockProvider::default();
    let cache = SettlementCache::new();
    let reqs = requirements(json!({}));
    let req = request(
        &reqs,
        &transfer_b64(FEE_PAYER, PAYER, PAY_TO, ASSET, AMOUNT),
    );
    let first = settle_request(&provider, AliasPolicy::Reject, &cache, &req).await;
    assert!(first.is_success());
    let second = settle_request(&provider, AliasPolicy::Reject, &cache, &req).await;
    match second {
        SettleResponse::Failure { reason, .. } => {
            assert_eq!(reason, ErrorReason::DuplicateSettlement);
        }
        other => panic!("{other:?}"),
    }
}
