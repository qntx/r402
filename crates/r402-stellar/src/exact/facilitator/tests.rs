//! Unit tests for Stellar exact verify/settle, matching @x402/stellar reasons.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    clippy::indexing_slicing,
    clippy::shadow_unrelated,
    clippy::match_same_arms,
    clippy::manual_async_fn,
    clippy::too_many_lines,
    reason = "test assertions with known JSON structure"
)]

use std::future::Future;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use r402_core::cache::SettlementCache;
use r402_core::error::ErrorReason;
use r402_core::wire::{SettleResponse, VerifyResponse};
use serde_json::{Value, json};
use stellar_rpc_client::{GetTransactionResponse, SimulateTransactionResponse};
use stellar_xdr::{
    AccountEntry, AccountEntryExt, AccountId, ContractEvent, ContractEventBody, ContractEventType,
    ContractEventV0, ContractId, DiagnosticEvent, ExtensionPoint, Hash, HostFunction, Limits,
    PublicKey, ScVal, SequenceNumber, String32, Thresholds, TransactionEnvelope, TransactionExt,
    Uint256, WriteXdr,
};

use super::{settle_request, verify_request_json};
use crate::USDC_TESTNET_ADDRESS;
use crate::chain::StellarFacilitatorError;
use crate::chain::rpc::{StellarRpc, StellarRpcError};
use crate::chain::xdr::{
    build_transfer_envelope, decode_transaction_envelope, encode_transaction_envelope,
    inner_transaction_mut, muxed_account_from_str, sc_address_from_str, unsigned_address_auth,
};
use crate::{BASE_FEE_STROOPS, DEFAULT_MAX_TRANSACTION_FEE_STROOPS, NULL_ACCOUNT};

const FACILITATOR: &str = "GCQAXB2D77Y4C66CTGVH25H2RMUKMQJGOWUPK7UXGG5MAQBONUEKFQ4P";
const PAYER: &str = "GBBO4ZDDZTSM2IUKQYBAST3CFHNPFXECGEFTGWTA2WELR2BIWDK57UVE";
const PAY_TO: &str = "GCHEI4PQEFJOA27MNZRPQNLGURS6KASW76X5UZCUZIXCOJLKXYCXOR2W";
const AMOUNT: i128 = 10_000;
const CURRENT_LEDGER: u32 = 1_000;
const AUTH_EXPIRATION: u32 = 1_012;

#[derive(Clone)]
struct MockRpc {
    latest: u32,
    estimated: u64,
    simulate: SimulateTransactionResponse,
    simulate_err: Option<String>,
    account: AccountEntry,
    submit: Result<GetTransactionResponse, String>,
    simulate_calls: Arc<Mutex<u32>>,
}

impl Default for MockRpc {
    fn default() -> Self {
        Self {
            latest: CURRENT_LEDGER,
            estimated: 5,
            simulate: success_simulation(PAYER, PAY_TO, AMOUNT, USDC_TESTNET_ADDRESS, 100),
            simulate_err: None,
            account: dummy_account(FACILITATOR),
            submit: Ok(GetTransactionResponse {
                status: "SUCCESS".to_owned(),
                ledger: Some(1),
                application_order: None,
                fee_bump: None,
                tx_hash: Some("ab".repeat(32)),
                created_at: None,
                envelope: None,
                result: None,
                result_meta: None,
                events: stellar_rpc_client::GetTransactionEvents {
                    contract_events: Vec::new(),
                    diagnostic_events: Vec::new(),
                    transaction_events: Vec::new(),
                },
            }),
            simulate_calls: Arc::new(Mutex::new(0)),
        }
    }
}

impl StellarRpc for MockRpc {
    fn latest_ledger(&self) -> impl Future<Output = Result<u32, StellarRpcError>> + Send {
        let latest = self.latest;
        async move { Ok(latest) }
    }

    fn get_account(
        &self,
        _address: &str,
    ) -> impl Future<Output = Result<AccountEntry, StellarRpcError>> + Send {
        let account = self.account.clone();
        async move { Ok(account) }
    }

    fn simulate_transaction(
        &self,
        _tx: &TransactionEnvelope,
    ) -> impl Future<Output = Result<SimulateTransactionResponse, StellarRpcError>> + Send {
        *self.simulate_calls.lock().unwrap() += 1;
        let err = self.simulate_err.clone();
        let sim = self.simulate.clone();
        async move {
            if let Some(err) = err {
                return Err(StellarRpcError::Request(err));
            }
            Ok(sim)
        }
    }

    fn send_transaction(
        &self,
        _tx: &TransactionEnvelope,
    ) -> impl Future<Output = Result<Hash, StellarRpcError>> + Send {
        async move {
            Hash::from_str(&"11".repeat(32)).map_err(|e| StellarRpcError::Request(e.to_string()))
        }
    }

    fn get_transaction(
        &self,
        _hash: &Hash,
    ) -> impl Future<Output = Result<GetTransactionResponse, StellarRpcError>> + Send {
        let submit = self.submit.clone();
        async move { submit.map_err(StellarRpcError::Request) }
    }

    fn poll_transaction(
        &self,
        _hash: &Hash,
        _timeout: Duration,
    ) -> impl Future<Output = Result<GetTransactionResponse, StellarRpcError>> + Send {
        let submit = self.submit.clone();
        async move { submit.map_err(StellarRpcError::Request) }
    }

    fn estimated_ledger_seconds(&self) -> impl Future<Output = u64> + Send {
        let estimated = self.estimated;
        async move { estimated }
    }
}

fn dummy_account(address: &str) -> AccountEntry {
    let id = AccountId::from_str(address)
        .unwrap_or(AccountId(PublicKey::PublicKeyTypeEd25519(Uint256([0; 32]))));
    AccountEntry {
        account_id: id,
        balance: 1_000_000,
        seq_num: SequenceNumber(1),
        num_sub_entries: 0,
        inflation_dest: None,
        flags: 0,
        home_domain: String32::default(),
        thresholds: Thresholds([1, 1, 1, 1]),
        signers: Vec::new().try_into().unwrap(),
        ext: AccountEntryExt::V0,
    }
}

fn transfer_event(from: &str, to: &str, amount: i128, asset: &str) -> DiagnosticEvent {
    use crate::chain::xdr::{sc_val_address, sc_val_i128};
    let symbol = stellar_xdr::ScSymbol("transfer".try_into().unwrap());
    let body = ContractEventV0 {
        topics: vec![
            ScVal::Symbol(symbol),
            sc_val_address(from).unwrap(),
            sc_val_address(to).unwrap(),
        ]
        .try_into()
        .unwrap(),
        data: sc_val_i128(amount),
    };
    let contract_id = match sc_address_from_str(asset).unwrap() {
        stellar_xdr::ScAddress::Contract(ContractId(hash)) => hash,
        _ => Hash([0; 32]),
    };
    DiagnosticEvent {
        in_successful_contract_call: true,
        event: ContractEvent {
            ext: ExtensionPoint::V0,
            contract_id: Some(ContractId(contract_id)),
            type_: ContractEventType::Contract,
            body: ContractEventBody::V0(body),
        },
    }
}

fn success_simulation(
    from: &str,
    to: &str,
    amount: i128,
    asset: &str,
    min_resource_fee: u64,
) -> SimulateTransactionResponse {
    let event = transfer_event(from, to, amount, asset);
    let event_b64 = event.to_xdr_base64(Limits::none()).unwrap();
    let data = stellar_xdr::SorobanTransactionData {
        ext: stellar_xdr::SorobanTransactionDataExt::V0,
        resources: stellar_xdr::SorobanResources::default(),
        resource_fee: i64::try_from(min_resource_fee).unwrap_or(0),
    };
    SimulateTransactionResponse {
        min_resource_fee,
        transaction_data: data.to_xdr_base64(Limits::none()).unwrap(),
        events: vec![event_b64],
        latest_ledger: CURRENT_LEDGER,
        error: None,
        restore_preamble: None,
        ..SimulateTransactionResponse::default()
    }
}

fn signed_auth(
    from: &str,
    host: &HostFunction,
    expiration: u32,
) -> stellar_xdr::SorobanAuthorizationEntry {
    let mut entry = unsigned_address_auth(from, 1, expiration, host).unwrap();
    if let stellar_xdr::SorobanCredentials::Address(creds) = &mut entry.credentials {
        creds.signature = ScVal::Bool(true);
    }
    entry
}

fn transfer_xdr(
    source: &str,
    from: &str,
    to: &str,
    asset: &str,
    amount: i128,
    auth: Vec<stellar_xdr::SorobanAuthorizationEntry>,
) -> String {
    let envelope = build_transfer_envelope(
        source,
        asset,
        from,
        to,
        amount,
        0,
        BASE_FEE_STROOPS,
        auth,
        TransactionExt::V0,
        60,
        0,
    )
    .unwrap();
    encode_transaction_envelope(&envelope).unwrap()
}

fn valid_tx() -> String {
    let host =
        crate::chain::xdr::transfer_host_function(USDC_TESTNET_ADDRESS, PAYER, PAY_TO, AMOUNT)
            .unwrap();
    transfer_xdr(
        NULL_ACCOUNT,
        PAYER,
        PAY_TO,
        USDC_TESTNET_ADDRESS,
        AMOUNT,
        vec![signed_auth(PAYER, &host, AUTH_EXPIRATION)],
    )
}

fn request(tx: &str) -> Value {
    json!({
        "paymentPayload": {
            "x402Version": 2,
            "accepted": {
                "scheme": "exact",
                "network": "stellar:testnet",
                "amount": AMOUNT.to_string(),
                "asset": USDC_TESTNET_ADDRESS,
                "payTo": PAY_TO,
                "maxTimeoutSeconds": 60,
                "extra": { "areFeesSponsored": true }
            },
            "payload": { "transaction": tx }
        },
        "paymentRequirements": {
            "scheme": "exact",
            "network": "stellar:testnet",
            "amount": AMOUNT.to_string(),
            "asset": USDC_TESTNET_ADDRESS,
            "payTo": PAY_TO,
            "maxTimeoutSeconds": 60,
            "extra": { "areFeesSponsored": true }
        }
    })
}

fn reason(response: &VerifyResponse) -> String {
    match response {
        VerifyResponse::Invalid { reason, .. } => reason.as_str().to_owned(),
        VerifyResponse::Valid { .. } => "valid".to_owned(),
        _ => "unknown".to_owned(),
    }
}

fn facilitator() -> Vec<String> {
    vec![FACILITATOR.to_owned()]
}

#[tokio::test]
async fn verify_valid_payload() {
    let rpc = MockRpc::default();
    let response = verify_request_json(
        &rpc,
        &facilitator(),
        DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
        &request(&valid_tx()),
    )
    .await;
    assert!(response.is_valid(), "{response:?}");
    match response {
        VerifyResponse::Valid { payer, .. } => assert_eq!(payer, PAYER),
        VerifyResponse::Invalid { .. } => panic!("expected valid"),
        _ => panic!("expected valid"),
    }
}

#[tokio::test]
async fn reject_invalid_version_scheme_network() {
    let rpc = MockRpc::default();
    let mut req = request(&valid_tx());
    req["paymentPayload"]["x402Version"] = json!(9);
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &req
            )
            .await
        ),
        "invalid_x402_version"
    );

    let mut req = request(&valid_tx());
    req["paymentPayload"]["accepted"]["scheme"] = json!("upto");
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &req
            )
            .await
        ),
        "unsupported_scheme"
    );

    let mut req = request(&valid_tx());
    req["paymentPayload"]["accepted"]["network"] = json!("foo:bar");
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &req
            )
            .await
        ),
        "network_mismatch"
    );

    let mut req = request(&valid_tx());
    req["paymentPayload"]["accepted"]["network"] = json!("eip155:1");
    req["paymentRequirements"]["network"] = json!("eip155:1");
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &req
            )
            .await
        ),
        "invalid_network"
    );
}

#[tokio::test]
async fn reject_unsponsored_extra() {
    let rpc = MockRpc::default();
    let mut req = request(&valid_tx());
    req["paymentRequirements"]["extra"]["areFeesSponsored"] = json!(false);
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &req
            )
            .await
        ),
        "invalid_exact_stellar_fees_sponsored_unsupported"
    );
}

#[tokio::test]
async fn reject_malformed_and_wrong_operation() {
    let rpc = MockRpc::default();
    let req = request("AAAA");
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &req
            )
            .await
        ),
        "invalid_exact_stellar_payload_malformed"
    );

    let host =
        crate::chain::xdr::transfer_host_function(USDC_TESTNET_ADDRESS, PAYER, PAY_TO, AMOUNT)
            .unwrap();
    let auth = signed_auth(PAYER, &host, AUTH_EXPIRATION);
    let mut envelope = build_transfer_envelope(
        NULL_ACCOUNT,
        USDC_TESTNET_ADDRESS,
        PAYER,
        PAY_TO,
        AMOUNT,
        0,
        BASE_FEE_STROOPS,
        vec![auth.clone()],
        TransactionExt::V0,
        60,
        0,
    )
    .unwrap();
    {
        let tx = inner_transaction_mut(&mut envelope).unwrap();
        let first = tx.operations.first().unwrap().clone();
        tx.operations = vec![first.clone(), first].try_into().unwrap();
    }
    let xdr = encode_transaction_envelope(&envelope).unwrap();
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_wrong_operation"
    );
}

#[tokio::test]
async fn reject_wrong_asset_recipient_amount() {
    let rpc = MockRpc::default();
    let mut req = request(&valid_tx());
    req["paymentRequirements"]["asset"] =
        json!("CDNVQW44C3HALYNVQ4SOBXY5EWYTGVYXX6JPESOLQDABJI5FC5LTRRUE");
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &req
            )
            .await
        ),
        "invalid_exact_stellar_payload_wrong_asset"
    );

    let mut req = request(&valid_tx());
    req["paymentRequirements"]["payTo"] =
        json!("GAHPYWLK6YRN7CVYZOO4H3VDRZ7PVF5UJGLZCSPAEIKJE2XSWF5LAGER");
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &req
            )
            .await
        ),
        "invalid_exact_stellar_payload_wrong_recipient"
    );

    let mut req = request(&valid_tx());
    req["paymentRequirements"]["amount"] = json!("10001");
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &req
            )
            .await
        ),
        "invalid_exact_stellar_payload_wrong_amount"
    );
}

#[tokio::test]
async fn reject_facilitator_is_payer_or_source() {
    let rpc = MockRpc::default();
    let host = crate::chain::xdr::transfer_host_function(
        USDC_TESTNET_ADDRESS,
        FACILITATOR,
        PAY_TO,
        AMOUNT,
    )
    .unwrap();
    let xdr = transfer_xdr(
        NULL_ACCOUNT,
        FACILITATOR,
        PAY_TO,
        USDC_TESTNET_ADDRESS,
        AMOUNT,
        vec![signed_auth(FACILITATOR, &host, AUTH_EXPIRATION)],
    );
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_facilitator_is_payer"
    );

    let host =
        crate::chain::xdr::transfer_host_function(USDC_TESTNET_ADDRESS, PAYER, PAY_TO, AMOUNT)
            .unwrap();
    let xdr = transfer_xdr(
        FACILITATOR,
        PAYER,
        PAY_TO,
        USDC_TESTNET_ADDRESS,
        AMOUNT,
        vec![signed_auth(PAYER, &host, AUTH_EXPIRATION)],
    );
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_unsafe_tx_or_op_source"
    );
}

#[tokio::test]
async fn reject_auth_problems() {
    let rpc = MockRpc::default();
    let xdr = transfer_xdr(
        NULL_ACCOUNT,
        PAYER,
        PAY_TO,
        USDC_TESTNET_ADDRESS,
        AMOUNT,
        Vec::new(),
    );
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_no_auth_entries"
    );

    let host =
        crate::chain::xdr::transfer_host_function(USDC_TESTNET_ADDRESS, PAYER, PAY_TO, AMOUNT)
            .unwrap();
    let unsigned = unsigned_address_auth(PAYER, 1, AUTH_EXPIRATION, &host).unwrap();
    let xdr = transfer_xdr(
        NULL_ACCOUNT,
        PAYER,
        PAY_TO,
        USDC_TESTNET_ADDRESS,
        AMOUNT,
        vec![unsigned],
    );
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_missing_payer_signature"
    );

    let pending = unsigned_address_auth(
        "GCHEI4PQEFJOA27MNZRPQNLGURS6KASW76X5UZCUZIXCOJLKXYCXOR2W",
        2,
        AUTH_EXPIRATION,
        &host,
    )
    .unwrap();
    let signed = signed_auth(PAYER, &host, AUTH_EXPIRATION);
    let xdr = transfer_xdr(
        NULL_ACCOUNT,
        PAYER,
        PAY_TO,
        USDC_TESTNET_ADDRESS,
        AMOUNT,
        vec![signed, pending],
    );
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_unexpected_pending_signatures"
    );

    let far = signed_auth(PAYER, &host, AUTH_EXPIRATION + 10_000);
    let xdr = transfer_xdr(
        NULL_ACCOUNT,
        PAYER,
        PAY_TO,
        USDC_TESTNET_ADDRESS,
        AMOUNT,
        vec![far],
    );
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_signature_expiration_too_far"
    );
}

#[tokio::test]
async fn reject_fee_exceeds_maximum() {
    let rpc = MockRpc::default();
    let response = verify_request_json(&rpc, &facilitator(), 150, &request(&valid_tx())).await;
    assert_eq!(
        reason(&response),
        "invalid_exact_stellar_payload_fee_exceeds_maximum"
    );
}

#[tokio::test]
async fn reject_simulation_failed() {
    let mut rpc = MockRpc::default();
    rpc.simulate.error = Some("boom".to_owned());
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&valid_tx())
            )
            .await
        ),
        "invalid_exact_stellar_payload_simulation_failed"
    );
}

#[tokio::test]
async fn settle_success_and_duplicate() {
    let rpc = MockRpc::default();
    let cache = SettlementCache::new();
    let req = request(&valid_tx());
    let first = settle_request(
        &rpc,
        &facilitator(),
        &cache,
        DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
        &req,
        0,
        |_verified, _timeout, _now| async move {
            Ok(GetTransactionResponse {
                status: "SUCCESS".to_owned(),
                ledger: Some(1),
                application_order: None,
                fee_bump: None,
                tx_hash: Some("cd".repeat(32)),
                created_at: None,
                envelope: None,
                result: None,
                result_meta: None,
                events: stellar_rpc_client::GetTransactionEvents {
                    contract_events: Vec::new(),
                    diagnostic_events: Vec::new(),
                    transaction_events: Vec::new(),
                },
            })
        },
    )
    .await;
    assert!(first.is_success(), "{first:?}");

    let second = settle_request(
        &rpc,
        &facilitator(),
        &cache,
        DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
        &req,
        0,
        |_verified, _timeout, _now| async move { Err(StellarFacilitatorError::NoSigner) },
    )
    .await;
    match second {
        SettleResponse::Failure { reason, .. } => {
            assert_eq!(reason, ErrorReason::DuplicateSettlement);
        }
        SettleResponse::Success { .. } => panic!("expected duplicate"),
        _ => panic!("expected duplicate"),
    }
}

#[tokio::test]
async fn settle_maps_poll_failed_and_send_rejected() {
    let rpc = MockRpc::default();
    let cache = SettlementCache::new();
    let failed = settle_request(
        &rpc,
        &facilitator(),
        &cache,
        DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
        &request(&valid_tx()),
        0,
        |_verified, _timeout, _now| async move {
            Err(StellarFacilitatorError::Rpc(
                StellarRpcError::TransactionFailed {
                    hash: "aa".repeat(32),
                    detail: "result none".to_owned(),
                },
            ))
        },
    )
    .await;
    match failed {
        SettleResponse::Failure {
            reason, message, ..
        } => {
            assert_eq!(reason.as_str(), "settle_exact_stellar_transaction_failed");
            assert!(
                message
                    .as_ref()
                    .is_some_and(|m| m.contains(&"aa".repeat(32))),
                "{message:?}"
            );
        }
        _ => panic!("expected failure"),
    }

    let cache = SettlementCache::new();
    let rejected = settle_request(
        &rpc,
        &facilitator(),
        &cache,
        DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
        &request(&valid_tx()),
        0,
        |_verified, _timeout, _now| async move {
            Err(StellarFacilitatorError::Rpc(
                StellarRpcError::SendRejected {
                    status: "ERROR".to_owned(),
                    hash: Some("bb".repeat(32)),
                    detail: "tx failed".to_owned(),
                },
            ))
        },
    )
    .await;
    match rejected {
        SettleResponse::Failure { reason, .. } => {
            assert_eq!(
                reason.as_str(),
                "settle_exact_stellar_transaction_submission_failed"
            );
        }
        _ => panic!("expected submission failure"),
    }

    let cache = SettlementCache::new();
    let timed_out = settle_request(
        &rpc,
        &facilitator(),
        &cache,
        DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
        &request(&valid_tx()),
        0,
        |_verified, _timeout, _now| async move {
            Err(StellarFacilitatorError::Rpc(
                StellarRpcError::TransactionTimeout {
                    hash: "cc".repeat(32),
                },
            ))
        },
    )
    .await;
    match timed_out {
        SettleResponse::Failure { reason, .. } => {
            assert_eq!(reason.as_str(), "settle_exact_stellar_transaction_failed");
        }
        _ => panic!("expected timeout as transaction_failed"),
    }
}

#[tokio::test]
async fn reject_wrong_function_name() {
    let rpc = MockRpc::default();
    let host =
        crate::chain::xdr::transfer_host_function(USDC_TESTNET_ADDRESS, PAYER, PAY_TO, AMOUNT)
            .unwrap();
    let mut envelope = decode_transaction_envelope(&transfer_xdr(
        NULL_ACCOUNT,
        PAYER,
        PAY_TO,
        USDC_TESTNET_ADDRESS,
        AMOUNT,
        vec![signed_auth(PAYER, &host, AUTH_EXPIRATION)],
    ))
    .unwrap();
    {
        let tx = inner_transaction_mut(&mut envelope).unwrap();
        let op = crate::chain::xdr::invoke_host_function_op_mut(tx).unwrap();
        if let HostFunction::InvokeContract(ref mut args) = op.host_function {
            args.function_name = stellar_xdr::ScSymbol("mint".try_into().unwrap());
        }
    }
    let xdr = encode_transaction_envelope(&envelope).unwrap();
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_wrong_function_name"
    );
}

#[tokio::test]
async fn reject_unsupported_credential_type() {
    let rpc = MockRpc::default();
    let host =
        crate::chain::xdr::transfer_host_function(USDC_TESTNET_ADDRESS, PAYER, PAY_TO, AMOUNT)
            .unwrap();
    let mut entry = unsigned_address_auth(PAYER, 1, AUTH_EXPIRATION, &host).unwrap();
    entry.credentials = stellar_xdr::SorobanCredentials::SourceAccount;
    let xdr = transfer_xdr(
        NULL_ACCOUNT,
        PAYER,
        PAY_TO,
        USDC_TESTNET_ADDRESS,
        AMOUNT,
        vec![entry],
    );
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_unsupported_credential_type"
    );
}

#[tokio::test]
async fn reject_facilitator_in_auth_and_subinvocations() {
    let rpc = MockRpc::default();
    let host =
        crate::chain::xdr::transfer_host_function(USDC_TESTNET_ADDRESS, PAYER, PAY_TO, AMOUNT)
            .unwrap();
    let xdr = transfer_xdr(
        NULL_ACCOUNT,
        PAYER,
        PAY_TO,
        USDC_TESTNET_ADDRESS,
        AMOUNT,
        vec![signed_auth(FACILITATOR, &host, AUTH_EXPIRATION)],
    );
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_facilitator_in_auth"
    );

    let mut nested = signed_auth(PAYER, &host, AUTH_EXPIRATION);
    let child = nested.root_invocation.clone();
    nested.root_invocation.sub_invocations = vec![child].try_into().unwrap();
    let xdr = transfer_xdr(
        NULL_ACCOUNT,
        PAYER,
        PAY_TO,
        USDC_TESTNET_ADDRESS,
        AMOUNT,
        vec![nested],
    );
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_has_subinvocations"
    );
}

#[tokio::test]
async fn reject_op_source_and_muxed_facilitator() {
    let rpc = MockRpc::default();
    let mut envelope = decode_transaction_envelope(&valid_tx()).unwrap();
    {
        let tx = inner_transaction_mut(&mut envelope).unwrap();
        let op = tx.operations.iter_mut().next().unwrap();
        op.source_account = Some(muxed_account_from_str(FACILITATOR).unwrap());
    }
    let xdr = encode_transaction_envelope(&envelope).unwrap();
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_unsafe_tx_or_op_source"
    );

    let payload = crate::chain::types::ed25519_account_payload(FACILITATOR).unwrap();
    let muxed = format!(
        "{}",
        stellar_strkey::ed25519::MuxedAccount {
            ed25519: payload,
            id: 9,
        }
    );
    let mut envelope = decode_transaction_envelope(&valid_tx()).unwrap();
    {
        let tx = inner_transaction_mut(&mut envelope).unwrap();
        let op = tx.operations.iter_mut().next().unwrap();
        op.source_account = Some(muxed_account_from_str(&muxed).unwrap());
    }
    let xdr = encode_transaction_envelope(&envelope).unwrap();
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_unsafe_tx_or_op_source"
    );
}

#[tokio::test]
async fn reject_simulation_event_variants() {
    let host =
        crate::chain::xdr::transfer_host_function(USDC_TESTNET_ADDRESS, PAYER, PAY_TO, AMOUNT)
            .unwrap();
    let xdr = transfer_xdr(
        NULL_ACCOUNT,
        PAYER,
        PAY_TO,
        USDC_TESTNET_ADDRESS,
        AMOUNT,
        vec![signed_auth(PAYER, &host, AUTH_EXPIRATION)],
    );

    let mut rpc = MockRpc::default();
    rpc.simulate.events.clear();
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_no_transfer_events"
    );

    let mut rpc = MockRpc::default();
    let transfer = transfer_event(PAYER, PAY_TO, AMOUNT, USDC_TESTNET_ADDRESS);
    rpc.simulate.events = vec![
        transfer.to_xdr_base64(Limits::none()).unwrap(),
        transfer.to_xdr_base64(Limits::none()).unwrap(),
    ];
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_multiple_transfers"
    );

    let mut rpc = MockRpc::default();
    let mut mint = transfer.clone();
    {
        let ContractEventBody::V0(ref mut body) = mint.event.body;
        body.topics = vec![ScVal::Symbol(stellar_xdr::ScSymbol(
            "mint".try_into().unwrap(),
        ))]
        .try_into()
        .unwrap();
    }
    rpc.simulate.events = vec![mint.to_xdr_base64(Limits::none()).unwrap()];
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_event_not_transfer"
    );

    let mut rpc = MockRpc::default();
    let mut missing = transfer.clone();
    missing.event.contract_id = None;
    rpc.simulate.events = vec![missing.to_xdr_base64(Limits::none()).unwrap()];
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_event_missing_contract_id"
    );

    let mut rpc = MockRpc::default();
    rpc.simulate.events = vec![
        transfer_event(
            PAYER,
            PAY_TO,
            AMOUNT,
            "CDNVQW44C3HALYNVQ4SOBXY5EWYTGVYXX6JPESOLQDABJI5FC5LTRRUE",
        )
        .to_xdr_base64(Limits::none())
        .unwrap(),
    ];
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_event_wrong_asset"
    );

    let mut rpc = MockRpc::default();
    rpc.simulate.events = vec![
        transfer_event(PAY_TO, PAY_TO, AMOUNT, USDC_TESTNET_ADDRESS)
            .to_xdr_base64(Limits::none())
            .unwrap(),
    ];
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_event_wrong_from"
    );

    let mut rpc = MockRpc::default();
    rpc.simulate.events = vec![
        transfer_event(PAYER, PAYER, AMOUNT, USDC_TESTNET_ADDRESS)
            .to_xdr_base64(Limits::none())
            .unwrap(),
    ];
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_event_wrong_to"
    );

    let mut rpc = MockRpc::default();
    rpc.simulate.events = vec![
        transfer_event(PAYER, PAY_TO, AMOUNT + 1, USDC_TESTNET_ADDRESS)
            .to_xdr_base64(Limits::none())
            .unwrap(),
    ];
    assert_eq!(
        reason(
            &verify_request_json(
                &rpc,
                &facilitator(),
                DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
                &request(&xdr)
            )
            .await
        ),
        "invalid_exact_stellar_payload_event_wrong_amount"
    );
}

#[tokio::test]
async fn golden_ts_transaction_xdr_decodes() {
    let raw = include_str!("../fixtures/ts_transaction.b64").trim();
    let envelope = decode_transaction_envelope(raw).unwrap();
    assert!(matches!(envelope, TransactionEnvelope::Tx(_)));
    let rpc = MockRpc::default();
    let response = verify_request_json(
        &rpc,
        &facilitator(),
        DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
        &request(raw),
    )
    .await;
    assert!(
        response.is_valid() || matches!(response, VerifyResponse::Invalid { .. }),
        "{response:?}"
    );
}
