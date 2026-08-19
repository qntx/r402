//! Unit tests for XRPL exact verify/settle.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unused_async,
    clippy::indexing_slicing,
    clippy::needless_pass_by_value,
    reason = "test assertions with known JSON structure"
)]

use r402_core::cache::SettlementCache;
use r402_core::error::ErrorReason;
use r402_core::wire::{SettleResponse, VerifyResponse};
use serde_json::{Value, json};

use super::{settle_request, verify_request_json};
use crate::DEFAULT_MAX_FEE_DROPS;
use crate::chain::codec::{invoice_id_to_field, sign_transaction};
use crate::chain::rpc::{
    XrplAccountAuthorization, XrplRpc, XrplRpcError, XrplSimulationResult, XrplSubmitResult,
    XrplTxResult,
};
use crate::exact::client::XrplSigner;

const PAY_TO: &str = "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh";
const ISSUER: &str = "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh";
const INVOICE: &str = "INV-2026-XRPL-001";
const AMOUNT: &str = "1000000";

#[derive(Clone)]
struct MockRpc {
    ledger: u32,
    sequence: u32,
    regular_key: Option<String>,
    master_disabled: bool,
    tickets: Vec<u32>,
    simulate: String,
    submit_hash: String,
    tx_validated: bool,
    tx_code: String,
    fail_account: bool,
}

impl Default for MockRpc {
    fn default() -> Self {
        Self {
            ledger: 990,
            sequence: 1,
            regular_key: None,
            master_disabled: false,
            tickets: vec![4],
            simulate: "tesSUCCESS".to_owned(),
            submit_hash: "AB".repeat(32),
            tx_validated: true,
            tx_code: "tesSUCCESS".to_owned(),
            fail_account: false,
        }
    }
}

impl XrplRpc for MockRpc {
    async fn current_ledger_index(&self) -> Result<u32, XrplRpcError> {
        Ok(self.ledger)
    }

    async fn account_authorization(
        &self,
        _account: &str,
    ) -> Result<XrplAccountAuthorization, XrplRpcError> {
        if self.fail_account {
            return Err(XrplRpcError::Rpc("account_info failed".to_owned()));
        }
        Ok(XrplAccountAuthorization {
            regular_key: self.regular_key.clone(),
            is_master_key_disabled: self.master_disabled,
            sequence: self.sequence,
        })
    }

    async fn ticket_sequences(&self, _account: &str) -> Result<Vec<u32>, XrplRpcError> {
        Ok(self.tickets.clone())
    }

    async fn fee_drops(&self) -> Result<u64, XrplRpcError> {
        Ok(12)
    }

    async fn simulate(&self, _unsigned_tx: &Value) -> Result<XrplSimulationResult, XrplRpcError> {
        Ok(XrplSimulationResult {
            engine_result: self.simulate.clone(),
            engine_result_message: None,
        })
    }

    async fn submit(&self, _signed_tx_blob: &str) -> Result<XrplSubmitResult, XrplRpcError> {
        Ok(XrplSubmitResult {
            hash: self.submit_hash.clone(),
            engine_result: "tesSUCCESS".to_owned(),
        })
    }

    async fn tx(&self, hash: &str) -> Result<Option<XrplTxResult>, XrplRpcError> {
        Ok(Some(XrplTxResult {
            hash: hash.to_owned(),
            validated: self.tx_validated,
            result_code: self.tx_code.clone(),
            meta: Some(json!({"TransactionResult": self.tx_code})),
        }))
    }
}

fn test_signer() -> XrplSigner {
    XrplSigner::from_seed("sEdTM1uX8pu2do5XvTnutH6HsouMaM2").expect("test seed")
}

fn sign_payment(overrides: Value) -> String {
    let signer = test_signer();
    let mut tx = json!({
        "TransactionType": "Payment",
        "Account": signer.classic_address(),
        "Destination": PAY_TO,
        "Amount": AMOUNT,
        "Fee": "12",
        "Sequence": 1,
        "LastLedgerSequence": 1000,
        "InvoiceID": invoice_id_to_field(INVOICE),
    });
    if let Some(obj) = overrides.as_object() {
        for (k, v) in obj {
            if v.is_null() {
                tx.as_object_mut().unwrap().remove(k);
            } else {
                tx[k] = v.clone();
            }
        }
    }
    sign_transaction(&mut tx, &signer_private(), signer.public_key()).expect("sign")
}

fn signer_private() -> String {
    let (public, private) =
        xrpl::core::keypairs::derive_keypair("sEdTM1uX8pu2do5XvTnutH6HsouMaM2", false)
            .expect("keypair");
    assert_eq!(public, test_signer().public_key());
    private
}

fn base_requirements() -> Value {
    json!({
        "scheme": "exact",
        "network": "xrpl:1",
        "asset": "XRP",
        "payTo": PAY_TO,
        "amount": AMOUNT,
        "maxTimeoutSeconds": 60,
        "extra": {
            "areFeesSponsored": false,
            "invoiceId": INVOICE
        }
    })
}

fn request(blob: &str, accepted: &Value, reqs: &Value) -> Value {
    json!({
        "x402Version": 2,
        "paymentPayload": {
            "x402Version": 2,
            "accepted": accepted,
            "payload": { "signedTxBlob": blob }
        },
        "paymentRequirements": reqs
    })
}

fn reason(response: &VerifyResponse) -> String {
    match response {
        VerifyResponse::Invalid { reason, .. } => reason.to_string(),
        other => panic!("expected invalid, got {other:?}"),
    }
}

#[tokio::test]
async fn verify_valid_xrp_payment() {
    let blob = sign_payment(json!({}));
    let reqs = base_requirements();
    let rpc = MockRpc::default();
    let response =
        verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, &request(&blob, &reqs, &reqs)).await;
    assert!(response.is_valid(), "{response:?}");
}

#[tokio::test]
async fn verify_rejects_wrong_version_scheme_network() {
    let blob = sign_payment(json!({}));
    let reqs = base_requirements();
    let rpc = MockRpc::default();

    let mut payload = request(&blob, &reqs, &reqs);
    payload["paymentPayload"]["x402Version"] = json!(1);
    assert!(
        reason(&verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, &payload).await)
            .contains("x402_version")
    );

    let mut reqs_bad = reqs.clone();
    reqs_bad["scheme"] = json!("upto");
    assert!(
        reason(
            &verify_request_json(
                &rpc,
                DEFAULT_MAX_FEE_DROPS,
                &request(&blob, &reqs, &reqs_bad)
            )
            .await
        )
        .contains("scheme")
    );

    let mut reqs_net = reqs.clone();
    reqs_net["network"] = json!("eip155:1");
    assert!(
        reason(
            &verify_request_json(
                &rpc,
                DEFAULT_MAX_FEE_DROPS,
                &request(&blob, &reqs, &reqs_net)
            )
            .await
        )
        .contains("network")
    );
}

#[tokio::test]
async fn verify_rejects_sponsored_fees() {
    let blob = sign_payment(json!({}));
    let mut reqs = base_requirements();
    reqs["extra"]["areFeesSponsored"] = json!(true);
    let rpc = MockRpc::default();
    assert!(
        reason(
            &verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, &request(&blob, &reqs, &reqs)).await
        )
        .contains("fees_sponsored")
    );
}

#[tokio::test]
async fn verify_rejects_destination_mismatch() {
    let blob = sign_payment(json!({ "Destination": test_signer().classic_address() }));
    let reqs = base_requirements();
    let rpc = MockRpc::default();
    assert!(
        reason(
            &verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, &request(&blob, &reqs, &reqs)).await
        )
        .contains("destination")
    );
}

#[tokio::test]
async fn verify_rejects_partial_payment_and_paths() {
    let blob = sign_payment(json!({ "Flags": crate::TF_PARTIAL_PAYMENT }));
    let reqs = base_requirements();
    let rpc = MockRpc::default();
    assert!(
        reason(
            &verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, &request(&blob, &reqs, &reqs)).await
        )
        .contains("partial_payment")
    );
}

#[tokio::test]
async fn verify_rejects_stale_sequence() {
    let blob = sign_payment(json!({ "Sequence": 9 }));
    let reqs = base_requirements();
    let rpc = MockRpc::default();
    assert!(
        reason(
            &verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, &request(&blob, &reqs, &reqs)).await
        )
        .contains("sequence_not_current")
    );
}

#[tokio::test]
async fn verify_rejects_ticket_on_sequence_method() {
    let blob = sign_payment(json!({ "TicketSequence": 4 }));
    let reqs = base_requirements();
    let rpc = MockRpc::default();
    assert!(
        reason(
            &verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, &request(&blob, &reqs, &reqs)).await
        )
        .contains("ticket_sequence_not_allowed")
    );
}

#[tokio::test]
async fn verify_rejects_missing_ticket() {
    let blob = sign_payment(json!({ "Sequence": 0, "TicketSequence": 99 }));
    let mut reqs = base_requirements();
    reqs["extra"]["assetTransferMethod"] = json!("ticketSequence");
    let rpc = MockRpc::default();
    assert!(
        reason(
            &verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, &request(&blob, &reqs, &reqs)).await
        )
        .contains("ticket_not_available")
    );
}

#[tokio::test]
async fn verify_rejects_simulation_failure() {
    let blob = sign_payment(json!({}));
    let reqs = base_requirements();
    let rpc = MockRpc {
        simulate: "tecUNFUNDED_PAYMENT".to_owned(),
        ..MockRpc::default()
    };
    assert!(
        reason(
            &verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, &request(&blob, &reqs, &reqs)).await
        )
        .contains("simulation_failed")
    );
}

#[tokio::test]
async fn verify_rejects_last_ledger_too_large() {
    let blob = sign_payment(json!({ "LastLedgerSequence": 50_000 }));
    let reqs = base_requirements();
    let rpc = MockRpc::default();
    assert!(
        reason(
            &verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, &request(&blob, &reqs, &reqs)).await
        )
        .contains("lastledgersequence_too_large")
    );
}

#[tokio::test]
async fn verify_rejects_fee_too_high() {
    let blob = sign_payment(json!({ "Fee": "99999" }));
    let reqs = base_requirements();
    let rpc = MockRpc::default();
    assert!(
        reason(
            &verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, &request(&blob, &reqs, &reqs)).await
        )
        .contains("fee_too_high")
    );
}

#[tokio::test]
async fn verify_rejects_invoice_mismatch() {
    let blob = sign_payment(json!({ "InvoiceID": "AA".repeat(32) }));
    let reqs = base_requirements();
    let rpc = MockRpc::default();
    assert!(
        reason(
            &verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, &request(&blob, &reqs, &reqs)).await
        )
        .contains("invoice_id_mismatch")
    );
}

#[tokio::test]
async fn verify_iou_requires_sendmax() {
    let signer = test_signer();
    let mut tx = json!({
        "TransactionType": "Payment",
        "Account": signer.classic_address(),
        "Destination": PAY_TO,
        "Amount": { "currency": "USD", "issuer": ISSUER, "value": "10.5" },
        "Fee": "12",
        "Sequence": 1,
        "LastLedgerSequence": 1000,
        "InvoiceID": invoice_id_to_field(INVOICE),
        "DestinationTag": 12345,
    });
    let blob = sign_transaction(&mut tx, &signer_private(), signer.public_key()).unwrap();
    let reqs = json!({
        "scheme": "exact",
        "network": "xrpl:1",
        "asset": "USD",
        "payTo": PAY_TO,
        "amount": "10.5",
        "maxTimeoutSeconds": 60,
        "extra": {
            "areFeesSponsored": false,
            "invoiceId": INVOICE,
            "issuer": ISSUER,
            "destinationTag": 12345
        }
    });
    let rpc = MockRpc::default();
    assert!(
        reason(
            &verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, &request(&blob, &reqs, &reqs)).await
        )
        .contains("sendmax_required")
    );
}

#[tokio::test]
async fn verify_iou_success() {
    let signer = test_signer();
    let amount = json!({ "currency": "USD", "issuer": ISSUER, "value": "10.5" });
    let mut tx = json!({
        "TransactionType": "Payment",
        "Account": signer.classic_address(),
        "Destination": PAY_TO,
        "Amount": amount,
        "SendMax": amount,
        "Fee": "12",
        "Sequence": 1,
        "LastLedgerSequence": 1000,
        "InvoiceID": invoice_id_to_field(INVOICE),
        "DestinationTag": 12345,
    });
    let blob = sign_transaction(&mut tx, &signer_private(), signer.public_key()).unwrap();
    let reqs = json!({
        "scheme": "exact",
        "network": "xrpl:1",
        "asset": "USD",
        "payTo": PAY_TO,
        "amount": "10.5",
        "maxTimeoutSeconds": 60,
        "extra": {
            "areFeesSponsored": false,
            "invoiceId": INVOICE,
            "issuer": ISSUER,
            "destinationTag": 12345
        }
    });
    let rpc = MockRpc::default();
    let response =
        verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, &request(&blob, &reqs, &reqs)).await;
    assert!(response.is_valid(), "{response:?}");
}

#[tokio::test]
async fn settle_success_and_duplicate() {
    let blob = sign_payment(json!({}));
    let reqs = base_requirements();
    let rpc = MockRpc::default();
    let cache = SettlementCache::new();
    let req = request(&blob, &reqs, &reqs);
    let first = settle_request(
        &rpc,
        &cache,
        DEFAULT_MAX_FEE_DROPS,
        &req,
        |_blob, _ll| async {
            Ok(XrplTxResult {
                hash: "AB".repeat(32),
                validated: true,
                result_code: "tesSUCCESS".to_owned(),
                meta: None,
            })
        },
    )
    .await;
    assert!(first.is_success(), "{first:?}");
    let second = settle_request(
        &rpc,
        &cache,
        DEFAULT_MAX_FEE_DROPS,
        &req,
        |_blob, _ll| async { panic!("duplicate must not submit") },
    )
    .await;
    match second {
        SettleResponse::Failure { reason, .. } => {
            assert_eq!(reason, ErrorReason::DuplicateSettlement);
        }
        other => panic!("expected duplicate, got {other:?}"),
    }
}

#[tokio::test]
async fn verify_rejects_network_id_on_standard_network() {
    let blob = sign_payment(json!({ "NetworkID": 1 }));
    let reqs = base_requirements();
    let rpc = MockRpc::default();
    assert!(
        reason(
            &verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, &request(&blob, &reqs, &reqs)).await
        )
        .contains("network_id_for_standard_network")
    );
}

#[tokio::test]
async fn verify_rejects_master_disabled_without_regular_key() {
    let blob = sign_payment(json!({}));
    let reqs = base_requirements();
    let rpc = MockRpc {
        master_disabled: true,
        ..MockRpc::default()
    };
    assert!(
        reason(
            &verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, &request(&blob, &reqs, &reqs)).await
        )
        .contains("signer_not_authorized")
    );
}

#[tokio::test]
async fn verify_rejects_missing_last_ledger() {
    let blob = sign_payment(json!({ "LastLedgerSequence": Value::Null }));
    let reqs = base_requirements();
    let rpc = MockRpc::default();
    let result = reason(
        &verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, &request(&blob, &reqs, &reqs)).await,
    );
    assert!(
        result.contains("lastledgersequence_missing") || result.contains("payload"),
        "{result}"
    );
}

#[tokio::test]
async fn verify_rejects_iou_issuer_missing() {
    let blob = sign_payment(json!({}));
    let mut reqs = base_requirements();
    reqs["asset"] = json!("USD");
    reqs["amount"] = json!("10.5");
    let rpc = MockRpc::default();
    assert!(
        reason(
            &verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, &request(&blob, &reqs, &reqs)).await
        )
        .contains("iou_issuer_missing")
    );
}

#[tokio::test]
async fn supported_signers_are_empty() {
    use crate::chain::{XrplChainProvider, XrplChainReference};
    use crate::exact::facilitator::XrplExactFacilitator;
    use r402_core::chain::ChainProvider;
    use r402_core::facilitator::Facilitator;

    let provider = XrplChainProvider::new(
        XrplChainReference::TESTNET,
        Some("http://127.0.0.1:1".to_owned()),
    )
    .unwrap();
    assert!(provider.signer_addresses().is_empty());
    let fac = XrplExactFacilitator::new(provider, DEFAULT_MAX_FEE_DROPS);
    let supported = fac.supported().await.unwrap();
    assert!(supported.signers.get("xrpl:*").is_some_and(Vec::is_empty));
    assert_eq!(
        supported.kinds[0].extra.as_ref().unwrap()["areFeesSponsored"],
        false
    );
}
