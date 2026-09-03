#![allow(
    unused_crate_dependencies,
    reason = "lib optional deps appear as unused externs"
)]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::unwrap_in_result,
    clippy::expect_used,
    clippy::missing_assert_message,
    clippy::unused_async_trait_impl,
    clippy::assigning_clones,
    clippy::panic,
    clippy::unused_async,
    clippy::field_reassign_with_default,
    clippy::needless_pass_by_value,
    clippy::excessive_nesting,
    reason = "idiomatic test-code patterns"
)]

//! In-process exact-scheme tests. No live rippled. No HTTP E2E.

use r402_protocol::scheme::SchemeId;
use r402_xrpl::chain::XrplChainReference;
use r402_xrpl::{RLUSD, XRP, XRPL_NETWORKS, XrplExact};

#[test]
fn crate_name_matches_directory() {
    assert_eq!(env!("CARGO_PKG_NAME"), "r402-xrpl");
}

#[test]
fn xrpl_exact_scheme_id() {
    assert_eq!(XrplExact.namespace(), "xrpl");
    assert_eq!(XrplExact.scheme(), "exact");
    assert_eq!(XrplExact.caip_family(), "xrpl:*");
}

#[test]
fn networks_table_has_mainnet_testnet_devnet() {
    assert_eq!(XRPL_NETWORKS.len(), 3);
    let names: Vec<_> = XRPL_NETWORKS.iter().map(|n| n.name).collect();
    assert_eq!(names, ["xrpl", "xrpl-testnet", "xrpl-devnet"]);
}

#[test]
fn default_xrp_and_rlusd_match_oracle() {
    assert_eq!(XRP::mainnet().decimals, r402_xrpl::XRP_DECIMALS);
    assert_eq!(RLUSD::mainnet().decimals, r402_xrpl::RLUSD_DECIMALS);
    assert_eq!(RLUSD::mainnet().asset.as_str(), r402_xrpl::RLUSD_CURRENCY);
}

#[test]
fn chain_reference_converts_to_caip2() {
    let chain: r402_protocol::ChainId = XrplChainReference::TESTNET.into();
    assert_eq!(chain.to_string(), "xrpl:1");
}

#[test]
fn extra_fixture_round_trips() {
    let json = include_str!("../../../tests/fixtures/xrpl/ts_payment_requirements.json");
    let reqs: r402_xrpl::exact::v2::PaymentRequirements = serde_json::from_str(json).unwrap();
    assert_eq!(reqs.network.to_string(), "xrpl:1");
    assert_eq!(reqs.asset.as_str(), "XRP");
    let extra = reqs.extra.unwrap();
    assert!(!extra.are_fees_sponsored);
    assert_eq!(extra.invoice_id.as_deref(), Some("INV-2026-XRPL-001"));
}

#[cfg(feature = "client")]
#[test]
fn find_default_xrpl_asset_covers_rlusd() {
    use r402_protocol::ChainId;
    use r402_xrpl::find_default_xrpl_asset;

    let mainnet: ChainId = "xrpl:0".parse().expect("mainnet");
    let info = find_default_xrpl_asset(r402_xrpl::RLUSD_CURRENCY, &mainnet).expect("RLUSD");
    assert_eq!(info.symbol, "RLUSD");
    assert_eq!(info.decimals, u32::from(r402_xrpl::RLUSD_DECIMALS));
    assert!(find_default_xrpl_asset("XRP", &mainnet).is_none());
}

#[cfg(feature = "server")]
#[test]
fn price_tag_is_two_arg() {
    use r402_xrpl::chain::XrplClassicAddress;

    let pay_to: XrplClassicAddress = "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".parse().unwrap();
    let tag = XrplExact::price_tag(pay_to, XRP::testnet().amount(1_000_000u64));
    assert_eq!(tag.requirements.scheme, "exact");
    assert_eq!(tag.requirements.network.to_string(), "xrpl:1");
    let extra = tag.requirements.extra.as_ref().unwrap();
    assert_eq!(extra["areFeesSponsored"], false);
}

#[cfg(feature = "facilitator")]
fn try_new_question_mark() -> Result<(), r402_protocol::FacilitatorError> {
    use r402_xrpl::chain::{XrplChainProvider, XrplChainReference};
    use r402_xrpl::exact::XrplExactFacilitator;

    let provider = XrplChainProvider::new(
        XrplChainReference::TESTNET,
        Some("http://127.0.0.1:1".to_owned()),
    )
    .expect("provider");
    let _fac = XrplExactFacilitator::try_new(provider)?;
    Ok(())
}

#[cfg(feature = "facilitator")]
#[test]
fn try_new_question_mark_compiles() {
    try_new_question_mark().expect("try_new is currently infallible");
}

#[cfg(feature = "facilitator")]
mod verify_settle {
    use r402_facilitator::Facilitator;
    use r402_protocol::error::ErrorReason;
    use r402_protocol::network::ChainProvider;
    use r402_protocol::payment::{SettleResponse, VerifyRequest, VerifyResponse};
    use r402_xrpl::DEFAULT_MAX_FEE_DROPS;
    use r402_xrpl::chain::codec::{invoice_id_to_field, sign_transaction};
    use r402_xrpl::chain::rpc::{
        XrplAccountAuthorization, XrplRpc, XrplRpcError, XrplSimulationResult, XrplSubmitResult,
        XrplTxResult,
    };
    use r402_xrpl::chain::{XrplChainProvider, XrplChainReference};
    use r402_xrpl::exact::facilitator::{
        XrplExactFacilitator, XrplSettlementCache, settle_request, verify_request_json,
    };
    use serde_json::{Value, json};
    use xrpl::core::keypairs::derive_keypair;

    const PAY_TO: &str = "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh";
    const ISSUER: &str = "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh";
    const INVOICE: &str = "INV-2026-XRPL-001";
    const AMOUNT: &str = "1000000";
    const SEED: &str = "sEdTM1uX8pu2do5XvTnutH6HsouMaM2";

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

        async fn simulate(
            &self,
            _unsigned_tx: &Value,
        ) -> Result<XrplSimulationResult, XrplRpcError> {
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

    fn signer_keys() -> (String, String, String) {
        let (public, private) = derive_keypair(SEED, false).expect("keypair");
        let address = xrpl::core::keypairs::derive_classic_address(&public).expect("address");
        (address, public, private)
    }

    fn sign_payment(overrides: Value) -> String {
        let (account, public, private) = signer_keys();
        let mut tx = json!({
            "TransactionType": "Payment",
            "Account": account,
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
        sign_transaction(&mut tx, &private, &public).expect("sign")
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
        let response = verify_request_json(
            &rpc,
            DEFAULT_MAX_FEE_DROPS,
            "xrpl:1",
            &request(&blob, &reqs, &reqs),
        )
        .await;
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
            reason(&verify_request_json(&rpc, DEFAULT_MAX_FEE_DROPS, "xrpl:1", &payload).await)
                .contains("x402_version")
        );

        let mut reqs_bad = reqs.clone();
        reqs_bad["scheme"] = json!("upto");
        assert!(
            reason(
                &verify_request_json(
                    &rpc,
                    DEFAULT_MAX_FEE_DROPS,
                    "xrpl:1",
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
                    "xrpl:1",
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
                &verify_request_json(
                    &rpc,
                    DEFAULT_MAX_FEE_DROPS,
                    "xrpl:1",
                    &request(&blob, &reqs, &reqs)
                )
                .await
            )
            .contains("fees_sponsored")
        );
    }

    #[tokio::test]
    async fn verify_rejects_destination_mismatch() {
        let (account, _, _) = signer_keys();
        let blob = sign_payment(json!({ "Destination": account }));
        let reqs = base_requirements();
        let rpc = MockRpc::default();
        assert!(
            reason(
                &verify_request_json(
                    &rpc,
                    DEFAULT_MAX_FEE_DROPS,
                    "xrpl:1",
                    &request(&blob, &reqs, &reqs)
                )
                .await
            )
            .contains("destination")
        );
    }

    #[tokio::test]
    async fn verify_rejects_partial_payment_and_paths() {
        let reqs = base_requirements();
        let rpc = MockRpc::default();
        let flags_blob = sign_payment(json!({ "Flags": r402_xrpl::TF_PARTIAL_PAYMENT }));
        assert!(
            reason(
                &verify_request_json(
                    &rpc,
                    DEFAULT_MAX_FEE_DROPS,
                    "xrpl:1",
                    &request(&flags_blob, &reqs, &reqs)
                )
                .await
            )
            .contains("partial_payment")
        );
        let paths_blob = sign_payment(json!({ "Paths": [[{"account": PAY_TO}]] }));
        assert!(
            reason(
                &verify_request_json(
                    &rpc,
                    DEFAULT_MAX_FEE_DROPS,
                    "xrpl:1",
                    &request(&paths_blob, &reqs, &reqs)
                )
                .await
            )
            .contains("paths_not_allowed")
        );
    }

    #[tokio::test]
    async fn verify_rejects_delegate_memos_delivermin_signers() {
        let reqs = base_requirements();
        let rpc = MockRpc::default();
        let cases = [
            (json!({ "Delegate": PAY_TO }), "delegate_not_allowed"),
            (
                json!({ "Memos": [{ "Memo": { "MemoData": "DEAD" } }] }),
                "memos_not_allowed",
            ),
            (json!({ "DeliverMin": "1" }), "delivermin_not_allowed"),
            (
                json!({
                    "Signers": [{
                        "Signer": {
                            "Account": PAY_TO,
                            "SigningPubKey": "ED00",
                            "TxnSignature": "00"
                        }
                    }]
                }),
                "multisig_not_supported",
            ),
        ];
        for (overrides, needle) in cases {
            let blob = sign_payment(overrides);
            let got = reason(
                &verify_request_json(
                    &rpc,
                    DEFAULT_MAX_FEE_DROPS,
                    "xrpl:1",
                    &request(&blob, &reqs, &reqs),
                )
                .await,
            );
            assert!(got.contains(needle), "expected {needle} in {got}");
        }
    }

    #[tokio::test]
    async fn verify_rejects_stale_sequence() {
        let blob = sign_payment(json!({ "Sequence": 9 }));
        let reqs = base_requirements();
        let rpc = MockRpc::default();
        assert!(
            reason(
                &verify_request_json(
                    &rpc,
                    DEFAULT_MAX_FEE_DROPS,
                    "xrpl:1",
                    &request(&blob, &reqs, &reqs)
                )
                .await
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
                &verify_request_json(
                    &rpc,
                    DEFAULT_MAX_FEE_DROPS,
                    "xrpl:1",
                    &request(&blob, &reqs, &reqs)
                )
                .await
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
                &verify_request_json(
                    &rpc,
                    DEFAULT_MAX_FEE_DROPS,
                    "xrpl:1",
                    &request(&blob, &reqs, &reqs)
                )
                .await
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
                &verify_request_json(
                    &rpc,
                    DEFAULT_MAX_FEE_DROPS,
                    "xrpl:1",
                    &request(&blob, &reqs, &reqs)
                )
                .await
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
                &verify_request_json(
                    &rpc,
                    DEFAULT_MAX_FEE_DROPS,
                    "xrpl:1",
                    &request(&blob, &reqs, &reqs)
                )
                .await
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
                &verify_request_json(
                    &rpc,
                    DEFAULT_MAX_FEE_DROPS,
                    "xrpl:1",
                    &request(&blob, &reqs, &reqs)
                )
                .await
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
                &verify_request_json(
                    &rpc,
                    DEFAULT_MAX_FEE_DROPS,
                    "xrpl:1",
                    &request(&blob, &reqs, &reqs)
                )
                .await
            )
            .contains("invoice_id_mismatch")
        );
    }

    #[tokio::test]
    async fn verify_iou_requires_sendmax() {
        let (account, public, private) = signer_keys();
        let mut tx = json!({
            "TransactionType": "Payment",
            "Account": account,
            "Destination": PAY_TO,
            "Amount": { "currency": "USD", "issuer": ISSUER, "value": "10.5" },
            "Fee": "12",
            "Sequence": 1,
            "LastLedgerSequence": 1000,
            "InvoiceID": invoice_id_to_field(INVOICE),
            "DestinationTag": 12345,
        });
        let blob = sign_transaction(&mut tx, &private, &public).unwrap();
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
                &verify_request_json(
                    &rpc,
                    DEFAULT_MAX_FEE_DROPS,
                    "xrpl:1",
                    &request(&blob, &reqs, &reqs)
                )
                .await
            )
            .contains("sendmax_required")
        );
    }

    #[tokio::test]
    async fn verify_iou_success() {
        let (account, public, private) = signer_keys();
        let amount = json!({ "currency": "USD", "issuer": ISSUER, "value": "10.5" });
        let mut tx = json!({
            "TransactionType": "Payment",
            "Account": account,
            "Destination": PAY_TO,
            "Amount": amount,
            "SendMax": amount,
            "Fee": "12",
            "Sequence": 1,
            "LastLedgerSequence": 1000,
            "InvoiceID": invoice_id_to_field(INVOICE),
            "DestinationTag": 12345,
        });
        let blob = sign_transaction(&mut tx, &private, &public).unwrap();
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
        let response = verify_request_json(
            &rpc,
            DEFAULT_MAX_FEE_DROPS,
            "xrpl:1",
            &request(&blob, &reqs, &reqs),
        )
        .await;
        assert!(response.is_valid(), "{response:?}");
    }

    #[tokio::test]
    async fn settle_success_and_duplicate() {
        let blob = sign_payment(json!({}));
        let reqs = base_requirements();
        let rpc = MockRpc::default();
        let cache = XrplSettlementCache::new();
        let req = request(&blob, &reqs, &reqs);
        let first = settle_request(
            &rpc,
            &cache,
            DEFAULT_MAX_FEE_DROPS,
            "xrpl:1",
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
            "xrpl:1",
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
                &verify_request_json(
                    &rpc,
                    DEFAULT_MAX_FEE_DROPS,
                    "xrpl:1",
                    &request(&blob, &reqs, &reqs)
                )
                .await
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
                &verify_request_json(
                    &rpc,
                    DEFAULT_MAX_FEE_DROPS,
                    "xrpl:1",
                    &request(&blob, &reqs, &reqs)
                )
                .await
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
            &verify_request_json(
                &rpc,
                DEFAULT_MAX_FEE_DROPS,
                "xrpl:1",
                &request(&blob, &reqs, &reqs),
            )
            .await,
        );
        assert!(
            result.contains("lastledgersequence_missing") || result.contains("payload"),
            "{result}"
        );
    }

    #[tokio::test]
    async fn verify_rejects_iou_missing_amount() {
        let blob = sign_payment(json!({ "Amount": Value::Null }));
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
                "issuer": ISSUER
            }
        });
        let rpc = MockRpc::default();
        assert!(
            reason(
                &verify_request_json(
                    &rpc,
                    DEFAULT_MAX_FEE_DROPS,
                    "xrpl:1",
                    &request(&blob, &reqs, &reqs)
                )
                .await
            )
            .contains("iou_amount")
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
                &verify_request_json(
                    &rpc,
                    DEFAULT_MAX_FEE_DROPS,
                    "xrpl:1",
                    &request(&blob, &reqs, &reqs)
                )
                .await
            )
            .contains("iou_issuer_missing")
        );
    }

    #[tokio::test]
    async fn try_new_supported_is_exact_on_provider_chain() {
        let provider = XrplChainProvider::new(
            XrplChainReference::TESTNET,
            Some("http://127.0.0.1:1".to_owned()),
        )
        .unwrap();
        assert!(provider.signer_addresses().is_empty());
        let fac = XrplExactFacilitator::try_new(provider).expect("try_new");
        let supported = fac.supported().await.unwrap();
        assert!(supported.signers.get("xrpl:*").is_some_and(Vec::is_empty));
        let kind = supported.kinds.first().expect("one kind");
        assert_eq!(kind.scheme, "exact");
        assert_eq!(kind.network, "xrpl:1");
        assert_eq!(kind.x402_version, 2);
        assert_eq!(kind.extra.as_ref().unwrap()["areFeesSponsored"], false);
    }

    #[tokio::test]
    async fn verify_rejects_mainnet_payload_on_testnet_provider() {
        let provider = XrplChainProvider::new(
            XrplChainReference::TESTNET,
            Some("http://127.0.0.1:1".to_owned()),
        )
        .unwrap();
        assert_eq!(provider.chain_id().to_string(), "xrpl:1");
        let fac = XrplExactFacilitator::try_new(provider).expect("try_new");
        let blob = sign_payment(json!({}));
        let mut reqs = base_requirements();
        reqs["network"] = json!("xrpl:0");
        let response = fac
            .verify(VerifyRequest::from(request(&blob, &reqs, &reqs)))
            .await
            .expect("verify");
        assert!(
            reason(&response).contains("network_mismatch"),
            "{response:?}"
        );
    }
}
