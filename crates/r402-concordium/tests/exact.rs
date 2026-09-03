#![allow(
    unused_crate_dependencies,
    reason = "lib optional deps appear as unused externs"
)]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    unknown_lints,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::unwrap_in_result,
    clippy::expect_used,
    clippy::missing_assert_message,
    clippy::assigning_clones,
    clippy::needless_pass_by_value,
    clippy::panic,
    clippy::unused_async_trait_impl,
    clippy::unused_async,
    clippy::cognitive_complexity,
    clippy::too_many_lines,
    reason = "idiomatic test-code patterns"
)]

//! In-process exact-scheme tests. No live Concordium node.

use r402_concordium::{
    CCD_ASSET_IDENTIFIER, CCD_DECIMALS, CONCORDIUM_MAINNET_CAIP2, CONCORDIUM_MAINNET_GRPC,
    CONCORDIUM_NETWORKS, CONCORDIUM_TESTNET_CAIP2, CONCORDIUM_TESTNET_GRPC,
    CONCORDIUM_WILDCARD_CAIP2, ConcordiumExact, DEFAULT_FINALIZATION_TIMEOUT_MS,
    MAX_EXPIRY_OFFSET_SECONDS, USDR_TOKEN_ID, address_matches_regex, get_concordium_grpc_url,
    get_explorer_account_url, get_explorer_tx_url, parse_grpc_url,
};
use r402_protocol::scheme::SchemeId;

const PAY_TO: &str = "2xBpaHottqhwFZURMZW4uZduQvpxNDSy46iXMYs9kceNGaPpZX";
#[cfg(any(feature = "client", feature = "facilitator"))]
const SENDER: &str = "3UrcxPQeYywasrPcYUcqhvFu3SB2vBBDjj7TsaRQ431vGiczYp";
#[cfg(any(feature = "client", feature = "facilitator"))]
const SPONSOR: &str = "2xdTv8awN1BjgYEw8W1BVXVtiEwG2b29U8KoZQqJrDuEqddseE";

#[test]
fn crate_name_matches_directory() {
    assert_eq!(env!("CARGO_PKG_NAME"), "r402-concordium");
}

#[test]
fn concordium_exact_scheme_id() {
    assert_eq!(ConcordiumExact::new().namespace(), "ccd");
    assert_eq!(ConcordiumExact::new().scheme(), "exact");
    assert_eq!(ConcordiumExact::new().caip_family(), "ccd:*");
}

#[test]
fn networks_and_constants_match_oracle() {
    assert_eq!(CONCORDIUM_NETWORKS.len(), 2);
    assert_eq!(
        CONCORDIUM_MAINNET_CAIP2,
        "ccd:9dd9ca4d19e9393877d2c44b70f89acb"
    );
    assert_eq!(
        CONCORDIUM_TESTNET_CAIP2,
        "ccd:4221332d34e1694168c2a0c0b3fd0f27"
    );
    assert_eq!(CONCORDIUM_WILDCARD_CAIP2, "ccd:*");
    assert_eq!(CCD_DECIMALS, 6);
    assert_eq!(CCD_ASSET_IDENTIFIER, "CCD");
    assert_eq!(USDR_TOKEN_ID, "USDR");
    assert_eq!(MAX_EXPIRY_OFFSET_SECONDS, 600);
    assert_eq!(DEFAULT_FINALIZATION_TIMEOUT_MS, 60_000);
    assert_eq!(
        get_concordium_grpc_url(CONCORDIUM_MAINNET_CAIP2).expect("main"),
        CONCORDIUM_MAINNET_GRPC
    );
    assert_eq!(
        get_concordium_grpc_url(CONCORDIUM_TESTNET_CAIP2).expect("test"),
        CONCORDIUM_TESTNET_GRPC
    );
    assert!(get_concordium_grpc_url("ccd:*").is_err());
    assert_eq!(
        get_explorer_tx_url(CONCORDIUM_MAINNET_CAIP2, "abc123").as_deref(),
        Some("https://ccdexplorer.io/mainnet/transaction/abc123")
    );
    assert_eq!(
        get_explorer_account_url(CONCORDIUM_TESTNET_CAIP2, "4FmiTW2L4Rv").as_deref(),
        Some("https://ccdexplorer.io/testnet/account/4FmiTW2L4Rv")
    );
    assert!(get_explorer_tx_url("ccd:unknown", "abc").is_none());
    let (host, port) = parse_grpc_url("grpc.testnet.concordium.com:20000");
    assert_eq!(host, "grpc.testnet.concordium.com");
    assert_eq!(port, 20_000);
    let (host_no_port, port_default) = parse_grpc_url("grpc.example.com:");
    assert_eq!(host_no_port, "grpc.example.com");
    assert_eq!(port_default, 20_000);
    let (host_bad, port_bad) = parse_grpc_url("grpc.example.com:abc");
    assert_eq!(host_bad, "grpc.example.com");
    assert_eq!(port_bad, 20_000);
}

#[test]
fn address_regex_oracle() {
    assert!(address_matches_regex(PAY_TO));
    assert!(address_matches_regex(
        "3kBx2h5Y2veb4hZvAE2c1Zr6DYJwWbPr9xQJJBPWyFnXHF9UuN"
    ));
    assert!(!address_matches_regex(
        "0FmiTW2L4RvCsSVTjFAavYvrgnPLGNj43eiwPYmbhNqtAcMbWW"
    ));
    assert!(!address_matches_regex("4Fmi"));
}

#[cfg(feature = "client")]
#[test]
fn find_default_concordium_asset_covers_usdr() {
    use r402_concordium::find_default_concordium_asset;
    use r402_protocol::ChainId;

    let mainnet: ChainId = CONCORDIUM_MAINNET_CAIP2.parse().expect("mainnet");
    let usdr = find_default_concordium_asset(USDR_TOKEN_ID, &mainnet).expect("USDR");
    assert_eq!(usdr.symbol, "USDR");
    assert_eq!(usdr.decimals, 6);
    assert!(find_default_concordium_asset("CCD", &mainnet).is_none());
}

#[cfg(feature = "server")]
#[test]
fn price_tag_is_two_arg() {
    use r402_concordium::USDR;
    use r402_concordium::chain::ConcordiumAddress;

    let pay_to: ConcordiumAddress = PAY_TO.parse().expect("generated checksum address");
    let tag = ConcordiumExact::price_tag(pay_to, USDR::concordium_testnet().amount(1_000_000));
    assert_eq!(tag.requirements.scheme, "exact");
    assert_eq!(
        tag.requirements.network.to_string(),
        CONCORDIUM_TESTNET_CAIP2
    );
    assert!(tag.requirements.extra.is_none());
}

#[cfg(feature = "facilitator")]
mod facilitator {
    use std::sync::Mutex;
    use std::time::Duration;

    use r402_concordium::chain::{
        AccountSnapshot, ConcordiumChainProvider, ConcordiumChainReference, ConcordiumGrpc,
        ConcordiumNode, ConcordiumRpcError, ConcordiumSigner,
    };
    use r402_concordium::exact::ConcordiumExactFacilitator;
    use r402_concordium::exact::payload::{TransactionInfo, TransactionStatus};
    use r402_facilitator::Facilitator;
    use r402_protocol::payment::{SettleRequest, VerifyRequest};
    use serde_json::json;

    use super::{SPONSOR, *};

    #[derive(Debug)]
    struct MockNode {
        snapshot: Mutex<AccountSnapshot>,
        token_decimals: u8,
        token_balance: Mutex<Result<Option<u128>, ()>>,
        send_hash: String,
        send_err: Mutex<Option<String>>,
        finalization: TransactionInfo,
    }

    impl Default for MockNode {
        fn default() -> Self {
            Self {
                snapshot: Mutex::new(AccountSnapshot {
                    nonce: Some(1),
                    amount_micro_ccd: Some(5_000_000),
                    info: None,
                    key_seed: None,
                }),
                token_decimals: 6,
                token_balance: Mutex::new(Ok(Some(1_000_000))),
                send_hash: "aa".repeat(32),
                send_err: Mutex::new(None),
                finalization: TransactionInfo {
                    tx_hash: "aa".repeat(32),
                    status: TransactionStatus::Finalized,
                    sender: SENDER.to_owned(),
                    recipient: Some(PAY_TO.to_owned()),
                    amount: Some("1000000".to_owned()),
                    asset: Some("CCD".to_owned()),
                },
            }
        }
    }

    impl ConcordiumNode for MockNode {
        async fn get_account(&self, _address: &str) -> Result<AccountSnapshot, ConcordiumRpcError> {
            Ok(self.snapshot.lock().expect("lock").clone())
        }

        async fn next_nonce(&self, _address: &str) -> Result<u64, ConcordiumRpcError> {
            Ok(self.snapshot.lock().expect("lock").nonce.unwrap_or(1))
        }

        async fn get_token_decimals(&self, _token_id: &str) -> Result<u8, ConcordiumRpcError> {
            Ok(self.token_decimals)
        }

        async fn get_token_balance(
            &self,
            _address: &str,
            _token_id: &str,
        ) -> Result<Option<u128>, ConcordiumRpcError> {
            let guard = self.token_balance.lock().expect("lock");
            match *guard {
                Ok(v) => Ok(v),
                Err(()) => Err(ConcordiumRpcError::Transport("rpc".to_owned())),
            }
        }

        async fn send_v1(
            &self,
            _tx: concordium_rust_sdk::base::transactions::AccountTransactionV1<
                concordium_rust_sdk::types::transactions::EncodedPayload,
            >,
        ) -> Result<String, ConcordiumRpcError> {
            if let Some(err) = self.send_err.lock().expect("lock").as_ref() {
                return Err(ConcordiumRpcError::Transport(err.clone()));
            }
            Ok(self.send_hash.clone())
        }

        async fn wait_finalized(
            &self,
            _tx_hash: &str,
            _timeout: Duration,
        ) -> Result<TransactionInfo, ConcordiumRpcError> {
            Ok(self.finalization.clone())
        }
    }

    fn dummy_signer(address: &str) -> ConcordiumSigner {
        ConcordiumSigner::from_secret(address, "11".repeat(32)).expect("signer")
    }

    fn facilitator() -> ConcordiumExactFacilitator<MockNode> {
        let provider = ConcordiumChainProvider::new(
            ConcordiumChainReference::TESTNET,
            vec![dummy_signer(SPONSOR)],
            MockNode::default(),
        );
        ConcordiumExactFacilitator::try_new(provider).expect("try_new")
    }

    fn expiry() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_secs()
            + 30
    }

    fn ccd_tx(overrides: serde_json::Value) -> serde_json::Value {
        let mut tx = json!({
            "version": 1,
            "header": {
                "sender": SENDER,
                "expiry": expiry(),
                "sponsor": { "account": SPONSOR, "numSignatures": 1 },
                "numSignatures": 1,
                "nonce": 1
            },
            "payload": { "type": "transfer", "toAddress": PAY_TO, "amount": "1000000" },
            "signatures": { "sender": { "0": { "0": "aa" } }, "sponsor": {} }
        });
        merge(&mut tx, overrides);
        tx
    }

    fn merge(base: &mut serde_json::Value, patch: serde_json::Value) {
        match (base, patch) {
            (serde_json::Value::Object(base), serde_json::Value::Object(patch)) => {
                for (k, v) in patch {
                    merge(base.entry(k).or_insert(serde_json::Value::Null), v);
                }
            }
            (slot, v) => *slot = v,
        }
    }

    fn verify_body(tx: serde_json::Value, req_extra: serde_json::Value) -> serde_json::Value {
        json!({
            "x402Version": 2,
            "paymentPayload": {
                "x402Version": 2,
                "accepted": { "scheme": "exact", "network": CONCORDIUM_TESTNET_CAIP2 },
                "payload": { "signedTransaction": tx }
            },
            "paymentRequirements": {
                "scheme": "exact",
                "network": CONCORDIUM_TESTNET_CAIP2,
                "amount": "1000000",
                "asset": "CCD",
                "payTo": PAY_TO,
                "maxTimeoutSeconds": 600,
                "extra": req_extra
            }
        })
    }

    async fn verify(body: serde_json::Value) -> r402_protocol::payment::VerifyResponse {
        let fac = facilitator();
        fac.verify(VerifyRequest::from(body)).await.expect("verify")
    }

    fn reason(resp: &r402_protocol::payment::VerifyResponse) -> String {
        match resp {
            r402_protocol::payment::VerifyResponse::Invalid { reason, .. } => {
                reason.as_ref().map(ToString::to_string).unwrap_or_default()
            }
            _ => String::new(),
        }
    }

    #[tokio::test]
    async fn try_new_supported_advertises_fee_payer() {
        let fac = facilitator();
        let supported = fac.supported().await.expect("supported");
        let kind = supported.kinds.first().expect("kind");
        assert_eq!(kind.scheme, "exact");
        assert_eq!(kind.network, CONCORDIUM_TESTNET_CAIP2);
        let extra = kind.extra.as_ref().expect("extra");
        assert_eq!(
            extra.get("feePayer").and_then(serde_json::Value::as_str),
            Some(SPONSOR)
        );
        assert!(!supported.signers.is_empty());
    }

    #[tokio::test]
    async fn rejects_missing_payload() {
        let body = json!({
            "x402Version": 2,
            "paymentPayload": {
                "x402Version": 2,
                "accepted": { "scheme": "exact", "network": CONCORDIUM_TESTNET_CAIP2 },
                "payload": null
            },
            "paymentRequirements": {
                "scheme": "exact",
                "network": CONCORDIUM_TESTNET_CAIP2,
                "amount": "1000000",
                "asset": "CCD",
                "payTo": PAY_TO,
                "maxTimeoutSeconds": 60,
                "extra": {}
            }
        });
        let resp = verify(body).await;
        assert!(!resp.is_valid());
        assert!(reason(&resp).contains("missing_payload"));
    }

    #[tokio::test]
    async fn rejects_wrong_version() {
        let resp = verify(verify_body(
            ccd_tx(json!({"version": 0})),
            json!({ "feePayer": SPONSOR }),
        ))
        .await;
        assert!(reason(&resp).contains("invalid_transaction_version"));
    }

    #[tokio::test]
    async fn rejects_sponsor_mismatch() {
        let resp = verify(verify_body(
            ccd_tx(json!({
                "header": { "sponsor": { "account": "WRONG_SPONSOR_ADDRESS_HERE_12345678901234567890", "numSignatures": 1 } }
            })),
            json!({ "feePayer": SPONSOR }),
        ))
        .await;
        assert!(reason(&resp).contains("sponsor_mismatch"));
    }

    #[tokio::test]
    async fn rejects_expired() {
        let resp = verify(verify_body(
            ccd_tx(json!({ "header": { "expiry": 1_000_000_000 } })),
            json!({ "feePayer": SPONSOR }),
        ))
        .await;
        assert!(reason(&resp).contains("transaction_expired"));
    }

    #[tokio::test]
    async fn rejects_unsupported_scheme() {
        let mut body = verify_body(ccd_tx(json!({})), json!({ "feePayer": SPONSOR }));
        body["paymentPayload"]["accepted"]["scheme"] = json!("per_request");
        let resp = verify(body).await;
        assert_eq!(reason(&resp), "unsupported_scheme");
    }

    #[tokio::test]
    async fn rejects_network_mismatch() {
        let mut body = verify_body(ccd_tx(json!({})), json!({ "feePayer": SPONSOR }));
        body["paymentPayload"]["accepted"]["network"] = json!(CONCORDIUM_MAINNET_CAIP2);
        let resp = verify(body).await;
        assert_eq!(reason(&resp), "network_mismatch");
    }

    #[tokio::test]
    async fn rejects_missing_sender() {
        let resp = verify(verify_body(
            ccd_tx(json!({ "header": { "sender": "" } })),
            json!({ "feePayer": SPONSOR }),
        ))
        .await;
        assert_eq!(reason(&resp), "missing_sender");
    }

    #[tokio::test]
    async fn rejects_invalid_sender() {
        let resp = verify(verify_body(
            ccd_tx(json!({ "header": { "sender": "not-a-valid-base58-address" } })),
            json!({ "feePayer": SPONSOR }),
        ))
        .await;
        assert_eq!(reason(&resp), "invalid_sender_address");
    }

    #[tokio::test]
    async fn rejects_missing_fee_payer() {
        let resp = verify(verify_body(ccd_tx(json!({})), json!({}))).await;
        assert_eq!(reason(&resp), "missing_fee_payer");
    }

    #[tokio::test]
    async fn rejects_unmanaged_fee_payer() {
        let other = "3kBx2h5Y2veb4hZvAE2c1Zr6DYJwWbPr9xQJJBPWyFnXHF9UuN";
        let resp = verify(verify_body(
            ccd_tx(json!({ "header": { "sponsor": { "account": other, "numSignatures": 1 } } })),
            json!({ "feePayer": other }),
        ))
        .await;
        assert_eq!(reason(&resp), "fee_payer_not_managed_by_facilitator");
    }

    #[tokio::test]
    async fn rejects_missing_sponsor_header() {
        let mut tx = ccd_tx(json!({}));
        tx["header"]["sponsor"] = json!({ "numSignatures": 1 });
        let resp = verify(verify_body(tx, json!({ "feePayer": SPONSOR }))).await;
        assert_eq!(reason(&resp), "missing_sponsor_in_header");
    }

    #[tokio::test]
    async fn rejects_expiry_too_far() {
        let far = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_secs()
            + 3600;
        let resp = verify(verify_body(
            ccd_tx(json!({ "header": { "expiry": far } })),
            json!({ "feePayer": SPONSOR }),
        ))
        .await;
        assert!(reason(&resp).contains("expiry_too_far_in_future"));
    }

    #[tokio::test]
    async fn rejects_sponsor_as_sender() {
        let resp = verify(verify_body(
            ccd_tx(json!({
                "header": { "sender": SPONSOR, "sponsor": { "account": SPONSOR, "numSignatures": 1 } }
            })),
            json!({ "feePayer": SPONSOR }),
        ))
        .await;
        assert_eq!(reason(&resp), "sponsor_as_sender");
    }

    #[tokio::test]
    async fn rejects_ccd_token_update_as_invalid_operations() {
        let mut tx = ccd_tx(json!({}));
        tx["payload"] = json!({ "type": "tokenUpdate", "tokenId": "EURR", "operations": "aa" });
        let resp = verify(verify_body(tx, json!({ "feePayer": SPONSOR }))).await;
        assert_eq!(reason(&resp), "invalid_token_operations");
    }

    #[tokio::test]
    async fn rejects_plt_with_transfer_payload() {
        let mut body = verify_body(ccd_tx(json!({})), json!({ "feePayer": SPONSOR }));
        body["paymentRequirements"]["asset"] = json!("EURR");
        let resp = verify(body).await;
        assert!(reason(&resp).contains("asset_type_mismatch"));
    }

    #[tokio::test]
    async fn rejects_missing_token_id() {
        let mut tx = ccd_tx(json!({}));
        tx["payload"] = json!({ "type": "tokenUpdate", "operations": "aa" });
        let mut body = verify_body(tx, json!({ "feePayer": SPONSOR }));
        body["paymentRequirements"]["asset"] = json!("EURR");
        let resp = verify(body).await;
        assert_eq!(reason(&resp), "missing_token_id");
    }

    #[tokio::test]
    async fn rejects_missing_recipient() {
        let mut tx = ccd_tx(json!({}));
        tx["payload"] = json!({ "type": "transfer", "amount": "1000000" });
        let resp = verify(verify_body(tx, json!({ "feePayer": SPONSOR }))).await;
        assert_eq!(reason(&resp), "missing_recipient");
    }

    #[tokio::test]
    async fn rejects_recipient_mismatch() {
        let other = "3kBx2h5Y2veb4hZvAE2c1Zr6DYJwWbPr9xQJJBPWyFnXHF9UuN";
        let resp = verify(verify_body(
            ccd_tx(json!({ "payload": { "type": "transfer", "toAddress": other, "amount": "1000000" } })),
            json!({ "feePayer": SPONSOR }),
        ))
        .await;
        assert_eq!(reason(&resp), "recipient_mismatch");
    }

    #[tokio::test]
    async fn rejects_missing_sender_signature() {
        let mut tx = ccd_tx(json!({}));
        tx["signatures"] = json!({ "sender": {}, "sponsor": {} });
        let resp = verify(verify_body(tx, json!({ "feePayer": SPONSOR }))).await;
        assert_eq!(reason(&resp), "missing_sender_signature");
    }

    #[tokio::test]
    async fn rejects_amount_mismatch() {
        let resp = verify(verify_body(
            ccd_tx(json!({ "payload": { "type": "transfer", "toAddress": PAY_TO, "amount": "999999" } })),
            json!({ "feePayer": SPONSOR }),
        ))
        .await;
        assert!(reason(&resp).contains("amount_mismatch"));
    }

    #[tokio::test]
    async fn rejects_invalid_required_amount() {
        let mut body = verify_body(ccd_tx(json!({})), json!({ "feePayer": SPONSOR }));
        body["paymentRequirements"]["amount"] = json!("not-a-number");
        let resp = verify(body).await;
        assert_eq!(reason(&resp), "invalid_required_amount");
    }

    #[tokio::test]
    async fn rejects_missing_signed_transaction() {
        let mut body = verify_body(ccd_tx(json!({})), json!({ "feePayer": SPONSOR }));
        body["paymentPayload"]["payload"] = json!({});
        let resp = verify(body).await;
        assert!(reason(&resp).contains("missing_signed_transaction"));
    }

    #[tokio::test]
    async fn structural_ok_fails_at_signature() {
        let resp = verify(verify_body(
            ccd_tx(json!({})),
            json!({ "feePayer": SPONSOR }),
        ))
        .await;
        assert!(!resp.is_valid());
        assert!(
            reason(&resp).contains("signature_verification_failed")
                || reason(&resp) == "invalid_sender_signature"
        );
    }

    #[test]
    fn try_new_rejects_empty_signers() {
        let provider = ConcordiumChainProvider::new(
            ConcordiumChainReference::TESTNET,
            vec![],
            MockNode::default(),
        );
        assert!(ConcordiumExactFacilitator::try_new(provider).is_err());
    }

    #[allow(dead_code, reason = "keeps ConcordiumGrpc in the test binary")]
    fn _grpc_typecheck() {
        let _: Option<ConcordiumGrpc> = None;
    }

    #[tokio::test]
    async fn settle_fails_when_verify_fails() {
        let fac = facilitator();
        let body = verify_body(ccd_tx(json!({})), json!({}));
        let resp = fac.settle(SettleRequest::from(body)).await.expect("settle");
        assert!(!resp.is_success());
    }

    #[test]
    fn transfer_with_memo_rebuild_keeps_memo() {
        use std::collections::BTreeMap;

        use r402_concordium::chain::tx::rebuild_for_verify;
        use r402_concordium::exact::payload::{
            SignableV1Signatures, SignableV1Transaction, SignableV1TransactionHeader,
            SignableV1TransactionPayload, SponsorHeader,
        };

        let tx = SignableV1Transaction {
            version: 1,
            header: SignableV1TransactionHeader {
                sender: SENDER.to_owned(),
                nonce: 1,
                expiry: json!(1_700_000_300),
                num_signatures: 1,
                execution_energy_amount: Some(300),
                sponsor: Some(SponsorHeader {
                    account: Some(SPONSOR.to_owned()),
                    address: None,
                    num_signatures: 1,
                }),
            },
            payload: SignableV1TransactionPayload::TransferWithMemo {
                to_address: Some(PAY_TO.to_owned()),
                amount: json!("1000000"),
                memo: Some("deadbeef".to_owned()),
            },
            signatures: SignableV1Signatures {
                sender: BTreeMap::from([(
                    "0".to_owned(),
                    BTreeMap::from([("0".to_owned(), "aa".to_owned())]),
                )]),
                sponsor: BTreeMap::new(),
            },
        };
        let rebuilt = rebuild_for_verify(&tx).expect("rebuild");
        match rebuilt.payload.decode().expect("decode") {
            concordium_rust_sdk::base::transactions::Payload::TransferWithMemo { memo, .. } => {
                assert_eq!(
                    memo.as_ref(),
                    hex::decode("deadbeef").expect("hex").as_slice()
                );
            }
            other => panic!("expected TransferWithMemo, got {other:?}"),
        }
    }

    #[test]
    fn plt_rebuild_preserves_operations_hex() {
        use r402_concordium::chain::tx::{build_plt_transfer, rebuild_for_verify};

        let signer = dummy_signer(SENDER);
        let built = build_plt_transfer(&signer, PAY_TO, "1000000", "USDR", 6, SPONSOR, 3, expiry())
            .expect("build");
        let wire_hex = match &built.payload {
            r402_concordium::exact::payload::SignableV1TransactionPayload::TokenUpdate {
                operations,
                ..
            } => operations.as_str().expect("hex").to_owned(),
            _ => panic!("expected tokenUpdate"),
        };
        let rebuilt = rebuild_for_verify(&built).expect("rebuild");
        match rebuilt.payload.decode().expect("decode") {
            concordium_rust_sdk::base::transactions::Payload::TokenUpdate { payload } => {
                assert_eq!(hex::encode(payload.operations.as_ref()), wire_hex);
            }
            other => panic!("expected TokenUpdate, got {other:?}"),
        }
    }

    #[cfg(feature = "client")]
    #[tokio::test]
    async fn submission_failed_releases_settlement_cache() {
        use r402_concordium::exact::client::create_signed_transaction;
        use r402_concordium::exact::payload::v2::PaymentRequirements;
        use r402_concordium::exact::payload::{ConcordiumExtra, ExactScheme};
        use r402_protocol::payment::SettleResponse;

        let sender = ConcordiumSigner::from_secret(SENDER, "22".repeat(32)).expect("sender");
        let node = MockNode::default();
        node.snapshot.lock().expect("lock").nonce = Some(7);
        node.snapshot.lock().expect("lock").key_seed = Some([0x22; 32]);
        *node.send_err.lock().expect("lock") = Some("boom".to_owned());
        let req = PaymentRequirements::new(
            ExactScheme,
            CONCORDIUM_TESTNET_CAIP2.parse().expect("net"),
            "1000000".into(),
            PAY_TO.into(),
            "CCD".into(),
            60,
        )
        .with_extra(ConcordiumExtra {
            fee_payer: Some(SPONSOR.parse().expect("sponsor")),
        });
        let signed = create_signed_transaction(&sender, &node, &req)
            .await
            .expect("sign");
        let tx = serde_json::to_value(&signed.signed_transaction).expect("json");
        let body = verify_body(tx, json!({ "feePayer": SPONSOR }));
        let provider = ConcordiumChainProvider::new(
            ConcordiumChainReference::TESTNET,
            vec![dummy_signer(SPONSOR)],
            node,
        );
        let fac = ConcordiumExactFacilitator::try_new(provider).expect("try_new");
        let first = fac
            .settle(SettleRequest::from(body.clone()))
            .await
            .expect("settle 1");
        assert!(!first.is_success());
        match &first {
            SettleResponse::Failure { reason, .. } => {
                assert!(reason.to_string().contains("submission_failed"));
            }
            _ => panic!("expected failure"),
        }
        let second = fac
            .settle(SettleRequest::from(body))
            .await
            .expect("settle 2");
        match &second {
            SettleResponse::Failure { reason, .. } => {
                assert!(
                    !reason.to_string().contains("duplicate"),
                    "cache must release on submission_failed, got {reason}"
                );
                assert!(reason.to_string().contains("submission_failed"));
            }
            _ => panic!("expected failure"),
        }
    }

    #[tokio::test]
    async fn rejects_string_expiry_as_missing_header_expiry() {
        let mut tx = ccd_tx(json!({}));
        tx["header"]["expiry"] = json!("1700000300");
        let resp = verify(verify_body(tx, json!({ "feePayer": SPONSOR }))).await;
        assert!(reason(&resp).contains("missing_header_expiry"));
    }

    #[tokio::test]
    async fn rejects_invalid_amount_format() {
        let mut tx = ccd_tx(json!({}));
        tx["payload"] =
            json!({ "type": "transfer", "toAddress": PAY_TO, "amount": "not-a-number" });
        let resp = verify(verify_body(tx, json!({ "feePayer": SPONSOR }))).await;
        assert_eq!(reason(&resp), "invalid_amount_format");
    }
}

#[cfg(feature = "client")]
mod client {
    use std::time::Duration;

    use r402_concordium::chain::{
        AccountSnapshot, ConcordiumNode, ConcordiumRpcError, ConcordiumSigner,
    };
    use r402_concordium::exact::client::create_signed_transaction;
    use r402_concordium::exact::payload::v2::PaymentRequirements;
    use r402_concordium::exact::payload::{
        ConcordiumExtra, ExactScheme, SignableV1TransactionPayload,
    };

    use super::{CONCORDIUM_TESTNET_CAIP2, PAY_TO, SENDER, SPONSOR};

    #[derive(Clone)]
    struct ClientNode {
        nonce: u64,
        decimals: u8,
    }

    impl ConcordiumNode for ClientNode {
        async fn get_account(&self, _address: &str) -> Result<AccountSnapshot, ConcordiumRpcError> {
            Ok(AccountSnapshot {
                nonce: Some(self.nonce),
                amount_micro_ccd: Some(5_000_000),
                info: None,
                key_seed: None,
            })
        }

        async fn next_nonce(&self, _address: &str) -> Result<u64, ConcordiumRpcError> {
            Ok(self.nonce)
        }

        async fn get_token_decimals(&self, _token_id: &str) -> Result<u8, ConcordiumRpcError> {
            Ok(self.decimals)
        }

        async fn get_token_balance(
            &self,
            _address: &str,
            _token_id: &str,
        ) -> Result<Option<u128>, ConcordiumRpcError> {
            Ok(Some(1_000_000))
        }

        async fn send_v1(
            &self,
            _tx: concordium_rust_sdk::base::transactions::AccountTransactionV1<
                concordium_rust_sdk::types::transactions::EncodedPayload,
            >,
        ) -> Result<String, ConcordiumRpcError> {
            Ok("aa".repeat(32))
        }

        async fn wait_finalized(
            &self,
            _tx_hash: &str,
            _timeout: Duration,
        ) -> Result<r402_concordium::exact::payload::TransactionInfo, ConcordiumRpcError> {
            Err(ConcordiumRpcError::Transport("unused".to_owned()))
        }
    }

    fn signer() -> ConcordiumSigner {
        ConcordiumSigner::from_secret(SENDER, "22".repeat(32)).expect("signer")
    }

    fn requirements(asset: &str, timeout: u64) -> PaymentRequirements {
        PaymentRequirements::new(
            ExactScheme,
            CONCORDIUM_TESTNET_CAIP2.parse().expect("net"),
            "1000000".into(),
            PAY_TO.into(),
            asset.into(),
            timeout,
        )
        .with_extra(ConcordiumExtra {
            fee_payer: Some(SPONSOR.parse().expect("sponsor")),
        })
    }

    #[tokio::test]
    async fn create_signed_ccd_transfer_has_empty_sponsor_slot() {
        let payload = create_signed_transaction(
            &signer(),
            &ClientNode {
                nonce: 7,
                decimals: 6,
            },
            &requirements("CCD", 60),
        )
        .await
        .expect("sign");
        let tx = payload.signed_transaction;
        assert_eq!(tx.version, 1);
        assert_eq!(tx.header.sender, SENDER);
        assert_eq!(tx.header.nonce, 7);
        assert_eq!(
            tx.header
                .sponsor
                .as_ref()
                .and_then(|s| s.resolved_address()),
            Some(SPONSOR)
        );
        assert!(tx.signatures.sponsor.is_empty());
        assert!(!tx.signatures.sender.is_empty());
        assert_eq!(tx.header.execution_energy_amount, Some(300));
        assert!(matches!(
            tx.payload,
            SignableV1TransactionPayload::Transfer { .. }
        ));
    }

    #[tokio::test]
    async fn create_signed_plt_transfer_fetches_decimals() {
        let payload = create_signed_transaction(
            &signer(),
            &ClientNode {
                nonce: 3,
                decimals: 6,
            },
            &requirements("USDR", 60),
        )
        .await
        .expect("sign");
        assert!(matches!(
            payload.signed_transaction.payload,
            SignableV1TransactionPayload::TokenUpdate { .. }
        ));
        assert_eq!(
            payload.signed_transaction.header.execution_energy_amount,
            Some(400)
        );
        assert!(payload.signed_transaction.signatures.sponsor.is_empty());
    }

    #[tokio::test]
    async fn create_signed_rejects_short_timeout() {
        let err = create_signed_transaction(
            &signer(),
            &ClientNode {
                nonce: 1,
                decimals: 6,
            },
            &requirements("CCD", 5),
        )
        .await
        .expect_err("timeout");
        assert!(err.to_string().contains("maxTimeoutSeconds"));
    }

    #[tokio::test]
    async fn create_signed_requires_fee_payer() {
        let req = PaymentRequirements::new(
            ExactScheme,
            CONCORDIUM_TESTNET_CAIP2.parse().expect("net"),
            "1000000".into(),
            PAY_TO.into(),
            "CCD".into(),
            60,
        );
        let err = create_signed_transaction(
            &signer(),
            &ClientNode {
                nonce: 1,
                decimals: 6,
            },
            &req,
        )
        .await
        .expect_err("feePayer");
        assert!(err.to_string().contains("feePayer"));
    }
}
