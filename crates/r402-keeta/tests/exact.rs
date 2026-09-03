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
    clippy::expect_used,
    clippy::missing_assert_message,
    clippy::unused_async_trait_impl,
    clippy::assigning_clones,
    clippy::panic,
    clippy::clone_on_copy,
    clippy::needless_pass_by_value,
    clippy::excessive_nesting,
    clippy::let_underscore_must_use,
    clippy::unused_async,
    unknown_lints,
    reason = "idiomatic test-code patterns"
)]

//! In-process exact-scheme tests. No live Keeta RPC. No HTTP E2E.

use r402_keeta::chain::KeetaChainReference;
use r402_keeta::{KEETA_NETWORKS, KeetaExact, USDC};
use r402_protocol::scheme::SchemeId;

#[test]
fn crate_name_matches_directory() {
    assert_eq!(env!("CARGO_PKG_NAME"), "r402-keeta");
}

#[test]
fn keeta_exact_scheme_id() {
    assert_eq!(KeetaExact.namespace(), "keeta");
    assert_eq!(KeetaExact.scheme(), "exact");
    assert_eq!(KeetaExact.caip_family(), "keeta:*");
}

#[test]
fn networks_table_has_mainnet_and_testnet() {
    assert_eq!(KEETA_NETWORKS.len(), 2);
    let names: Vec<_> = KEETA_NETWORKS.iter().map(|n| n.name).collect();
    assert_eq!(names, ["keeta", "keeta-testnet"]);
}

#[test]
fn default_usdc_addresses_match_oracle() {
    assert_eq!(USDC::all().len(), 2);
    assert_eq!(
        USDC::keeta().address.as_str(),
        "keeta_amnkge74xitii5dsobstldatv3irmyimujfjotftx7plaaaseam4bntb7wnna"
    );
    assert_eq!(
        USDC::keeta_testnet().address.as_str(),
        "keeta_apna75yhhvnv4ei7ape55hndk4yepno7a7i2mhtiwahiygixjcnmvswxhnmnk"
    );
}

#[test]
fn chain_reference_converts_to_caip2() {
    let chain: r402_protocol::ChainId = KeetaChainReference::TESTNET.into();
    assert_eq!(chain.to_string(), "keeta:1413829460");
}

#[test]
fn spec_fixture_round_trips() {
    let raw = include_str!("../../../tests/fixtures/keeta/ts_payment_payload.json");
    let payload: r402_keeta::exact::payload::v2::PaymentPayload =
        serde_json::from_str(raw).expect("wire PaymentPayload");
    assert_eq!(payload.accepted.network.to_string(), "keeta:1413829460");
    assert_eq!(payload.accepted.scheme.to_string(), "exact");
    assert_eq!(payload.accepted.amount.as_str(), "1000000000");
    assert_eq!(
        payload.accepted.asset.as_str(),
        "keeta_anyiff4v34alvumupagmdyosydeq24lc4def5mrpmmyhx3j6vj2uucckeqn52"
    );
    assert_eq!(
        payload.accepted.pay_to.as_str(),
        "keeta_aabravistgwbrkpl4euafuiualiwcemgvv2hnu7ci66i76naj4vm6tmeahmzria"
    );
    assert_eq!(payload.accepted.max_timeout_seconds, 60);
    assert!(payload.accepted.extra.is_none());
    let expected_block = include_str!("../../../tests/fixtures/keeta/ts_block.b64").trim();
    assert_eq!(payload.payload.block, expected_block);
}

#[test]
fn spec_der_round_trips_through_block_try_from() {
    let encoded = include_str!("../../../tests/fixtures/keeta/ts_block.b64").trim();
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
        .expect("spec fixture is valid base64");
    let block = keetanetwork_block::Block::try_from(bytes.as_slice())
        .expect("spec DER must decode and verify via Block::try_from");
    assert_eq!(
        block.to_bytes(),
        bytes.as_slice(),
        "Block::to_bytes must round-trip the TS/spec DER"
    );
}

#[cfg(feature = "client")]
#[test]
fn find_default_keeta_asset_covers_usdc() {
    use r402_keeta::find_default_keeta_asset;
    use r402_protocol::ChainId;

    let mainnet: ChainId = "keeta:21378".parse().expect("mainnet");
    let usdc = find_default_keeta_asset(USDC::keeta().address.as_str(), &mainnet).expect("USDC");
    assert_eq!(usdc.symbol, "USDC");
    assert_eq!(usdc.decimals, 6);
    assert!(find_default_keeta_asset("not-usdc", &mainnet).is_none());
}

#[cfg(feature = "server")]
#[test]
fn price_tag_is_two_arg() {
    use keetanetwork_account::{
        Account, Accountable, GenericAccount, KeyED25519, KeyPairType, Keyable,
    };
    use r402_keeta::chain::KeetaAddress;

    let account = Account::<KeyED25519>::try_from(Accountable::KeyAndType(
        Keyable::from([1u8; 32]),
        KeyPairType::ED25519,
    ))
    .unwrap();
    let pay_to: KeetaAddress = GenericAccount::Ed25519(account)
        .to_string()
        .parse()
        .unwrap();
    let tag = KeetaExact::price_tag(pay_to, USDC::keeta_testnet().amount(1_000_000u128));
    assert_eq!(tag.requirements.scheme, "exact");
    assert_eq!(tag.requirements.network.to_string(), "keeta:1413829460");
    assert!(tag.requirements.extra.is_none());
}

#[cfg(feature = "facilitator")]
fn try_new_question_mark() -> Result<(), r402_protocol::FacilitatorError> {
    use r402_keeta::chain::{KeetaChainProvider, KeetaFeePayer};
    use r402_keeta::exact::KeetaExactFacilitator;

    let payer = KeetaFeePayer::from_ed25519_seed([2u8; 32], 0)
        .map_err(r402_protocol::FacilitatorError::internal)?;
    let provider = KeetaChainProvider::new(KeetaChainReference::TESTNET, vec![payer])
        .map_err(r402_protocol::FacilitatorError::internal)?;
    let _fac = KeetaExactFacilitator::try_new(provider)?;
    Ok(())
}

#[cfg(feature = "facilitator")]
#[test]
fn try_new_question_mark_compiles() {
    try_new_question_mark().expect("try_new is currently infallible once the provider is built");
}

#[cfg(feature = "facilitator")]
#[tokio::test]
async fn try_new_supported_is_exact_on_provider_chain() {
    use r402_facilitator::Facilitator;
    use r402_keeta::chain::{KeetaChainProvider, KeetaFeePayer};
    use r402_keeta::exact::KeetaExactFacilitator;

    let payer = KeetaFeePayer::from_ed25519_seed([2u8; 32], 0).expect("fee payer");
    let fee_payer = payer.address();
    let provider = KeetaChainProvider::new(KeetaChainReference::TESTNET, vec![payer])
        .expect("provider constructs from bundled network registry");
    let fac = KeetaExactFacilitator::try_new(provider).expect("try_new");
    let supported = fac.supported().await.expect("supported");
    let kind = supported.kinds.first().expect("one kind");
    assert_eq!(kind.scheme, "exact");
    assert_eq!(kind.network, "keeta:1413829460");
    assert_eq!(kind.x402_version, 2);
    assert!(
        !supported.signers.is_empty(),
        "try_new provider must advertise signers"
    );
    let signers = supported.signers.get("keeta:*").expect("keeta:* signers");
    assert_eq!(
        signers.first().map(compact_str::CompactString::as_str),
        Some(fee_payer.as_str())
    );
}

#[cfg(feature = "facilitator")]
mod verify_settle {
    use std::sync::Arc;
    use std::time::Duration;

    use keetanetwork_account::{
        Account, Accountable, GenericAccount, KeyED25519, KeyPairType, Keyable,
    };
    use keetanetwork_block::{
        AccountRef, Amount, Block, BlockBuilder, BlockHash, Hashable, Send, SetRep,
    };
    use r402_facilitator::SettlementCache;
    use r402_keeta::chain::KeetaChainReference;
    use r402_keeta::chain::provider::{KeetaAclView, KeetaPreflight, KeetaRpcError};
    use r402_keeta::exact::facilitator::{
        QueueError, SettlementQueue, settle_request, verify_request_json,
    };
    use r402_protocol::error::ErrorReason;
    use r402_protocol::payment::{SettleResponse, VerifyResponse};
    use serde_json::{Value, json};

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

    const fn testnet() -> u32 {
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
        let first = settle_request(
            &MockPreflight::default(),
            &[],
            &SettlementCache::new(),
            &queue,
            &req,
        )
        .await;
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
        let decoded = r402_keeta::exact::facilitator::decode_block(&encoded).unwrap();
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
        let first = tokio::spawn(async move { queue_first.enqueue(first_block).await });
        started.notified().await;
        let second = queue.enqueue(decoded).await;
        assert!(matches!(second, Err(QueueError::Duplicate)));
        release.notify_one();
        assert!(first.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn dropped_enqueue_does_not_pin_hash_after_worker_finishes() {
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
        let decoded = r402_keeta::exact::facilitator::decode_block(&encoded).unwrap();
        let started = Arc::new(tokio::sync::Notify::new());
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let release_rx = Arc::new(std::sync::Mutex::new(Some(release_rx)));
        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
        let done_tx = Arc::new(std::sync::Mutex::new(Some(done_tx)));
        let started_cb = Arc::clone(&started);
        let queue = Arc::new(SettlementQueue::mock(
            vec!["fee-payer".to_owned()],
            move |_addr, block| {
                let started_inner = Arc::clone(&started_cb);
                let release_rx = Arc::clone(&release_rx);
                let done_tx = Arc::clone(&done_tx);
                Box::pin(async move {
                    started_inner.notify_one();
                    let release = release_rx
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take();
                    if let Some(rx) = release {
                        let _ = rx.await;
                    }
                    let done = done_tx
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take();
                    if let Some(tx) = done {
                        let _ = tx.send(());
                    }
                    Ok(block.hash().to_string())
                })
            },
        ));
        let queue_first = Arc::clone(&queue);
        let first_block = decoded.clone();
        let first = tokio::spawn(async move { queue_first.enqueue(first_block).await });
        started.notified().await;
        first.abort();
        let in_flight = queue.enqueue(decoded.clone()).await;
        assert!(matches!(in_flight, Err(QueueError::Duplicate)));
        let _ = release_tx.send(());
        let _ = done_rx.await;
        let retried = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match queue.enqueue(decoded.clone()).await {
                    Ok(hash) => return Ok(hash),
                    Err(QueueError::Duplicate) => tokio::task::yield_now().await,
                    Err(err) => return Err(err),
                }
            }
        })
        .await
        .expect("worker must drop the hash after transmit");
        assert!(
            retried.is_ok(),
            "abandoned waiter must not pin the hash: {retried:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn completing_job_does_not_clear_retry_reservation() {
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
        let decoded = r402_keeta::exact::facilitator::decode_block(&encoded).unwrap();
        let first_started = Arc::new(tokio::sync::Notify::new());
        let first_release = Arc::new(tokio::sync::Notify::new());
        let retry_started = Arc::new(tokio::sync::Notify::new());
        let retry_release = Arc::new(tokio::sync::Notify::new());
        let first_started_cb = Arc::clone(&first_started);
        let first_release_cb = Arc::clone(&first_release);
        let retry_started_cb = Arc::clone(&retry_started);
        let retry_release_cb = Arc::clone(&retry_release);
        let queue = Arc::new(SettlementQueue::mock(
            vec!["fee-payer-1".to_owned(), "fee-payer-2".to_owned()],
            move |addr, block| {
                if addr == "fee-payer-1" {
                    hold_until(
                        Arc::clone(&first_started_cb),
                        Arc::clone(&first_release_cb),
                        block,
                    )
                } else {
                    hold_until(
                        Arc::clone(&retry_started_cb),
                        Arc::clone(&retry_release_cb),
                        block,
                    )
                }
            },
        ));
        let queue_waiter = Arc::clone(&queue);
        let first_block = decoded.clone();
        let retry_block = decoded.clone();
        let waiter = tokio::spawn(async move {
            queue_waiter
                .enqueue(first_block)
                .await
                .expect("first settle");
            queue_waiter.enqueue(retry_block).await
        });
        first_started.notified().await;
        first_release.notify_one();
        retry_started.notified().await;
        let third = queue.enqueue(decoded).await;
        assert!(
            matches!(third, Err(QueueError::Duplicate)),
            "Job::Drop must not clear a retry that already reserved the hash: {third:?}"
        );
        retry_release.notify_one();
        assert!(waiter.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn same_payer_jobs_do_not_overlap_submit() {
        let actors = actors();
        let first_encoded = signed_send(
            &actors.payer,
            &actors.payer,
            &actors.token,
            &actors.pay_to,
            "100",
            None,
            testnet(),
        );
        let second_encoded = signed_send(
            &actors.payer,
            &actors.payer,
            &actors.token,
            &actors.pay_to,
            "200",
            None,
            testnet(),
        );
        let first_block = r402_keeta::exact::facilitator::decode_block(&first_encoded).unwrap();
        let second_block = r402_keeta::exact::facilitator::decode_block(&second_encoded).unwrap();
        let first_hash = first_block.hash();
        let first_started = Arc::new(tokio::sync::Notify::new());
        let first_release = Arc::new(tokio::sync::Notify::new());
        let second_started = Arc::new(tokio::sync::Notify::new());
        let second_release = Arc::new(tokio::sync::Notify::new());
        let first_started_cb = Arc::clone(&first_started);
        let first_release_cb = Arc::clone(&first_release);
        let second_started_cb = Arc::clone(&second_started);
        let second_release_cb = Arc::clone(&second_release);
        let queue = Arc::new(SettlementQueue::mock(
            vec!["fee-payer-1".to_owned(), "fee-payer-2".to_owned()],
            move |_addr, block| {
                if block.hash() == first_hash {
                    hold_until(
                        Arc::clone(&first_started_cb),
                        Arc::clone(&first_release_cb),
                        block,
                    )
                } else {
                    hold_until(
                        Arc::clone(&second_started_cb),
                        Arc::clone(&second_release_cb),
                        block,
                    )
                }
            },
        ));
        let queue_a = Arc::clone(&queue);
        let job_a = tokio::spawn(async move { queue_a.enqueue(first_block).await });
        first_started.notified().await;
        let queue_b = Arc::clone(&queue);
        let job_b = tokio::spawn(async move { queue_b.enqueue(second_block).await });
        let second_started_early =
            tokio::time::timeout(Duration::from_millis(50), second_started.notified()).await;
        assert!(
            second_started_early.is_err(),
            "same-payer second submit must wait for the first transmit"
        );
        first_release.notify_one();
        second_started.notified().await;
        second_release.notify_one();
        assert!(job_a.await.unwrap().is_ok());
        assert!(job_b.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn different_payers_submit_in_parallel() {
        let actors = actors();
        let first_encoded = signed_send(
            &actors.payer,
            &actors.payer,
            &actors.token,
            &actors.pay_to,
            "100",
            None,
            testnet(),
        );
        let second_encoded = signed_send(
            &actors.other,
            &actors.other,
            &actors.token,
            &actors.pay_to,
            "200",
            None,
            testnet(),
        );
        let first_block = r402_keeta::exact::facilitator::decode_block(&first_encoded).unwrap();
        let second_block = r402_keeta::exact::facilitator::decode_block(&second_encoded).unwrap();
        let first_started = Arc::new(tokio::sync::Notify::new());
        let first_release = Arc::new(tokio::sync::Notify::new());
        let second_started = Arc::new(tokio::sync::Notify::new());
        let second_release = Arc::new(tokio::sync::Notify::new());
        let first_started_cb = Arc::clone(&first_started);
        let first_release_cb = Arc::clone(&first_release);
        let second_started_cb = Arc::clone(&second_started);
        let second_release_cb = Arc::clone(&second_release);
        let queue = Arc::new(SettlementQueue::mock(
            vec!["fee-payer-1".to_owned(), "fee-payer-2".to_owned()],
            move |addr, block| {
                if addr == "fee-payer-1" {
                    hold_until(
                        Arc::clone(&first_started_cb),
                        Arc::clone(&first_release_cb),
                        block,
                    )
                } else {
                    hold_until(
                        Arc::clone(&second_started_cb),
                        Arc::clone(&second_release_cb),
                        block,
                    )
                }
            },
        ));
        let queue_a = Arc::clone(&queue);
        let job_a = tokio::spawn(async move { queue_a.enqueue(first_block).await });
        let queue_b = Arc::clone(&queue);
        let job_b = tokio::spawn(async move { queue_b.enqueue(second_block).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            first_started.notified().await;
            second_started.notified().await;
        })
        .await
        .expect("different payers must start transmit concurrently");
        first_release.notify_one();
        second_release.notify_one();
        assert!(job_a.await.unwrap().is_ok());
        assert!(job_b.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn settle_replay_after_success_is_duplicate_settlement() {
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
        let cache = SettlementCache::new();
        let req = request(&actors, &block, json!({}));
        let first = settle_request(&MockPreflight::default(), &[], &cache, &queue, &req).await;
        assert!(matches!(first, SettleResponse::Success { .. }), "{first:?}");
        let second = settle_request(&MockPreflight::default(), &[], &cache, &queue, &req).await;
        match second {
            SettleResponse::Failure { reason, .. } => {
                assert_eq!(reason, ErrorReason::DuplicateSettlement);
            }
            other => panic!("expected DuplicateSettlement, got {other:?}"),
        }
    }

    fn hold_until(
        started: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
        block: Block,
    ) -> r402_facilitator::BoxFuture<'static, Result<String, String>> {
        Box::pin(async move {
            started.notify_one();
            release.notified().await;
            Ok(block.hash().to_string())
        })
    }
}
