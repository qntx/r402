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

//! In-process exact-scheme tests. No live Toncenter. No HTTP E2E.

use r402_protocol::scheme::SchemeId;
use r402_tvm::{TVM_NETWORKS, TvmExact, USDT, USDT_MAINNET_MINTER, USDT_TESTNET_MINTER};

#[test]
fn crate_name_matches_directory() {
    assert_eq!(env!("CARGO_PKG_NAME"), "r402-tvm");
}

#[test]
fn tvm_exact_scheme_id() {
    assert_eq!(TvmExact.namespace(), "tvm");
    assert_eq!(TvmExact.scheme(), "exact");
    assert_eq!(TvmExact.caip_family(), "tvm:*");
}

#[test]
fn networks_table_has_mainnet_and_testnet() {
    assert_eq!(TVM_NETWORKS.len(), 2);
    let names: Vec<_> = TVM_NETWORKS.iter().map(|n| n.name).collect();
    assert_eq!(names, ["ton", "ton-testnet"]);
}

#[test]
fn default_usdt_minters_match_oracle() {
    assert_eq!(USDT::all().len(), 2);
    assert_eq!(USDT::tvm().address.as_str(), USDT_MAINNET_MINTER);
    assert_eq!(USDT::tvm_testnet().address.as_str(), USDT_TESTNET_MINTER);
}

#[test]
fn chain_reference_converts_to_caip2() {
    let chain: r402_protocol::ChainId = r402_tvm::chain::TvmChainReference::TESTNET.into();
    assert_eq!(chain.to_string(), "tvm:-3");
}

#[test]
fn extra_fixture_round_trips() {
    let json = include_str!("../../../tests/fixtures/tvm/extra_sponsored.json");
    let extra: r402_tvm::exact::TvmExtra = serde_json::from_str(json).unwrap();
    assert!(extra.are_fees_sponsored);
    let encoded = serde_json::to_value(&extra).unwrap();
    assert_eq!(encoded, serde_json::json!({ "areFeesSponsored": true }));
}

#[cfg(feature = "client")]
#[test]
fn find_default_tvm_asset_covers_usdt() {
    use r402_protocol::ChainId;
    use r402_tvm::find_default_tvm_asset;

    let mainnet: ChainId = "tvm:-239".parse().expect("mainnet");
    let usdt = find_default_tvm_asset(USDT_MAINNET_MINTER, &mainnet).expect("USDT");
    assert_eq!(usdt.symbol, "USDT");
    assert_eq!(usdt.decimals, 6);
    assert!(
        find_default_tvm_asset(
            "0:0000000000000000000000000000000000000000000000000000000000000000",
            &mainnet
        )
        .is_none()
    );
}

#[cfg(feature = "server")]
#[test]
fn price_tag_is_two_arg() {
    use r402_tvm::chain::TvmAddress;

    let pay_to: TvmAddress = USDT_TESTNET_MINTER.parse().unwrap();
    let tag = TvmExact::price_tag(pay_to, USDT::tvm_testnet().amount(10_000u128));
    assert_eq!(tag.requirements.scheme, "exact");
    assert_eq!(tag.requirements.network.to_string(), "tvm:-3");
    assert!(tag.requirements.extra.is_none());
}

#[cfg(feature = "facilitator")]
fn try_new_question_mark() -> Result<(), r402_protocol::FacilitatorError> {
    use r402_tvm::chain::{HighloadV3Config, TvmChainProvider, TvmChainReference};
    use r402_tvm::exact::TvmExactFacilitator;

    let config = HighloadV3Config::from_private_key(&[7u8; 32]).expect("key");
    let provider = TvmChainProvider::new(TvmChainReference::TESTNET, config).expect("provider");
    let _fac = TvmExactFacilitator::try_new(provider)?;
    Ok(())
}

#[cfg(feature = "facilitator")]
#[test]
fn try_new_question_mark_compiles() {
    try_new_question_mark().expect("try_new is currently infallible");
}

#[cfg(feature = "facilitator")]
#[tokio::test]
async fn try_new_supported_is_exact_on_provider_chain() {
    use r402_facilitator::Facilitator;
    use r402_tvm::chain::{HighloadV3Config, TvmChainProvider, TvmChainReference};
    use r402_tvm::exact::TvmExactFacilitator;

    let config = HighloadV3Config::from_private_key(&[7u8; 32]).expect("key");
    let provider = TvmChainProvider::new(TvmChainReference::TESTNET, config).expect("provider");
    let fac = TvmExactFacilitator::try_new(provider).expect("try_new");
    let supported = fac.supported().await.expect("supported");
    let kind = supported.kinds.first().expect("one kind");
    assert_eq!(kind.scheme, "exact");
    assert_eq!(kind.network, "tvm:-3");
    assert_eq!(kind.x402_version, 2);
    assert!(
        !supported.signers.is_empty(),
        "try_new provider must advertise signers"
    );
    let extra = kind.extra.as_ref().expect("areFeesSponsored extra");
    assert_eq!(extra["areFeesSponsored"], true);
}

#[cfg(all(feature = "client", feature = "facilitator"))]
mod verify_settle {
    use std::time::{SystemTime, UNIX_EPOCH};

    use r402_facilitator::SettlementCache;
    use r402_protocol::payment::VerifyResponse;
    use r402_tvm::chain::codec::cell::cell_hash_base64;
    use r402_tvm::chain::codec::jetton::build_jetton_transfer_body;
    use r402_tvm::chain::codec::w5::{parse_exact_tvm_payload, parse_w5_init_data};
    use r402_tvm::chain::rpc::{TvmAccountState, TvmJettonWalletData, TvmRpc, TvmRpcError};
    use r402_tvm::chain::{TvmAddress, TvmRelayRequest};
    use r402_tvm::exact::client::TvmW5Signer;
    use r402_tvm::exact::error::{
        ERR_EXACT_TVM_FACILITATOR_INSUFFICIENT_BALANCE, ERR_EXACT_TVM_INSUFFICIENT_BALANCE,
        ERR_EXACT_TVM_INVALID_AMOUNT, ERR_EXACT_TVM_INVALID_ASSET, ERR_EXACT_TVM_INVALID_CODE_HASH,
        ERR_EXACT_TVM_INVALID_RECIPIENT, ERR_EXACT_TVM_INVALID_SEQNO,
        ERR_EXACT_TVM_INVALID_WALLET_ID, ERR_EXACT_TVM_SIMULATION_FAILED,
        ERR_EXACT_TVM_UNSUPPORTED_NETWORK, ERR_EXACT_TVM_UNSUPPORTED_SCHEME,
        ERR_EXACT_TVM_UNSUPPORTED_VERSION,
    };
    use r402_tvm::exact::facilitator::verify_response_json;
    use r402_tvm::exact::payload::TvmExtra;
    use r402_tvm::{
        DEFAULT_JETTON_WALLET_MESSAGE_AMOUNT, MIN_FACILITATOR_TON_BALANCE, USDT_TESTNET_MINTER,
        W5R1_CODE_HEX,
    };
    use serde_json::{Value, json};

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
            let addr: TvmAddress =
                address
                    .parse()
                    .map_err(|e: r402_tvm::chain::TvmAddressFormatError| {
                        TvmRpcError::Parse(e.to_string())
                    })?;
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
                        .and_then(|b| {
                            b.store_u32(32, self.payer_wallet_id.unwrap_or(parsed.wallet_id))
                        })
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
                .map_err(|e: r402_tvm::chain::TvmAddressFormatError| {
                    TvmRpcError::Parse(e.to_string())
                })
        }

        async fn get_jetton_wallet_data(
            &self,
            address: &str,
        ) -> Result<TvmJettonWalletData, TvmRpcError> {
            Ok(TvmJettonWalletData {
                address: address
                    .parse()
                    .map_err(|e: r402_tvm::chain::TvmAddressFormatError| {
                        TvmRpcError::Parse(e.to_string())
                    })?,
                balance: self.jetton_balance,
                owner: fixture_signer().address().clone(),
                jetton_minter: USDT_TESTNET_MINTER.parse().map_err(
                    |e: r402_tvm::chain::TvmAddressFormatError| TvmRpcError::Parse(e.to_string()),
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
        TvmW5Signer::from_seed(&[7u8; 32], r402_tvm::chain::TvmChainReference::TESTNET).unwrap()
    }

    fn signer_state_init(signer: &TvmW5Signer) -> r402_tvm::chain::codec::w5::StateInitCells {
        r402_tvm::chain::codec::w5::build_w5r1_state_init(
            &{
                // public key is inside state init data; rebuild from seed
                let s =
                    TvmW5Signer::from_seed(&[7u8; 32], r402_tvm::chain::TvmChainReference::TESTNET)
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
        let body =
            build_jetton_transfer_body(amount.parse().unwrap(), &pay_to_addr, &extra).unwrap();
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
                            "hash": r402_tvm::chain::codec::cell::cell_hash_base64_from_ton_hash(&settlement.transfer.body_hash)
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
            VerifyResponse::Invalid { reason, .. } => {
                reason.as_ref().map_or("", |r| r.as_str()).to_owned()
            }
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
            VerifyResponse::Valid { payer: p, .. } => {
                assert_eq!(p.as_deref(), Some(payer.as_str()))
            }
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
        assert_eq!(cache.reserve("abc"), r402_facilitator::Duplicate::No);
        assert_eq!(cache.reserve("abc"), r402_facilitator::Duplicate::Yes);
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
}
