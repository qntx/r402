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
    clippy::clone_on_copy,
    reason = "idiomatic test-code patterns"
)]

//! In-process exact-scheme tests. No live RPC. No HTTP E2E.

use r402_near::chain::{NearChainReference, USDC_MAINNET_ACCOUNT};
use r402_near::{NEAR_NETWORKS, NearExact, USDC};
use r402_protocol::scheme::SchemeId;

#[test]
fn crate_name_matches_directory() {
    assert_eq!(env!("CARGO_PKG_NAME"), "r402-near");
}

#[test]
fn near_exact_scheme_id() {
    assert_eq!(NearExact.namespace(), "near");
    assert_eq!(NearExact.scheme(), "exact");
    assert_eq!(NearExact.caip_family(), "near:*");
}

#[test]
fn networks_table_has_mainnet_and_testnet() {
    assert_eq!(NEAR_NETWORKS.len(), 2);
    let names: Vec<_> = NEAR_NETWORKS.iter().map(|n| n.name).collect();
    assert_eq!(names, ["near", "near-testnet"]);
}

#[test]
fn default_usdc_accounts_match_oracle() {
    assert_eq!(USDC::all().len(), 2);
    assert_eq!(USDC::near().address.as_str(), USDC_MAINNET_ACCOUNT);
    assert_eq!(
        USDC::near_testnet().address.as_str(),
        r402_near::chain::USDC_TESTNET_ACCOUNT
    );
}

#[test]
fn chain_reference_converts_to_caip2() {
    let chain: r402_protocol::ChainId = NearChainReference::TESTNET.into();
    assert_eq!(chain.to_string(), "near:testnet");
}

#[cfg(feature = "client")]
#[test]
fn find_default_near_asset_covers_usdc() {
    use r402_near::find_default_near_asset;
    use r402_protocol::ChainId;

    let mainnet: ChainId = "near:mainnet".parse().expect("mainnet");
    let usdc = find_default_near_asset(USDC_MAINNET_ACCOUNT, &mainnet).expect("USDC");
    assert_eq!(usdc.symbol, "USDC");
    assert_eq!(usdc.decimals, 6);
    assert!(find_default_near_asset("other.testnet", &mainnet).is_none());
}

#[cfg(feature = "server")]
#[test]
fn price_tag_is_two_arg() {
    use r402_near::chain::NearAddress;

    let pay_to: NearAddress = "merchant.testnet".parse().unwrap();
    let tag = NearExact::price_tag(pay_to, USDC::near_testnet().amount(1_000_000u128));
    assert_eq!(tag.requirements.scheme, "exact");
    assert_eq!(tag.requirements.network.to_string(), "near:testnet");
    assert!(tag.requirements.extra.is_none());
}

#[cfg(feature = "facilitator")]
fn try_new_question_mark(
    provider: r402_near::NearChainProvider,
) -> Result<(), r402_protocol::FacilitatorError> {
    let _fac = r402_near::NearExactFacilitator::try_new(provider)?;
    Ok(())
}

#[cfg(feature = "facilitator")]
#[test]
fn try_new_question_mark_compiles() {
    use near_crypto::{KeyType, SecretKey};
    use r402_near::chain::{NearChainProvider, NearRelayer};

    let secret = SecretKey::from_seed(KeyType::ED25519, "relayer.testnet");
    let relayer =
        NearRelayer::from_secret_key("relayer.testnet", secret.to_string()).expect("relayer");
    let provider = NearChainProvider::new(
        NearChainReference::TESTNET,
        vec![relayer],
        Some("https://example.invalid".to_owned()),
    );
    try_new_question_mark(provider).expect("try_new is currently infallible");
}

#[cfg(feature = "facilitator")]
#[tokio::test]
async fn try_new_supported_is_exact_on_provider_chain() {
    use near_crypto::{KeyType, SecretKey};
    use r402_facilitator::Facilitator;
    use r402_near::chain::{NearChainProvider, NearRelayer};
    use r402_near::exact::NearExactFacilitator;

    let secret = SecretKey::from_seed(KeyType::ED25519, "relayer.testnet");
    let relayer =
        NearRelayer::from_secret_key("relayer.testnet", secret.to_string()).expect("relayer");
    let provider = NearChainProvider::new(
        NearChainReference::TESTNET,
        vec![relayer],
        Some("https://example.invalid".to_owned()),
    );
    let fac = NearExactFacilitator::try_new(provider).expect("try_new");
    let supported = fac.supported().await.expect("supported");
    let kind = supported.kinds.first().expect("one kind");
    assert_eq!(kind.scheme, "exact");
    assert_eq!(kind.network, "near:testnet");
    assert_eq!(kind.x402_version, 2);
    assert!(
        !supported.signers.is_empty(),
        "try_new provider must advertise signers"
    );
    assert!(kind.extra.is_none(), "relayer is facilitator-local");
}

#[cfg(feature = "facilitator")]
mod verify_settle {
    use std::collections::HashMap;
    use std::future::Future;

    use near_crypto::{InMemorySigner, KeyType, Signer};
    use near_primitives::action::delegate::{
        DelegateAction, NonDelegateAction, SignedDelegateAction,
    };
    use near_primitives::action::{Action, FunctionCallAction, TransferAction};
    use near_primitives::gas::Gas;
    use near_primitives::types::{AccountId, Balance};
    use r402_facilitator::SettlementCache;
    use r402_near::chain::rpc::{
        NearAccessKeyPermissionKind, NearAccessKeyView, NearAccountView, NearReceiptStatus,
        NearRpc, NearRpcError, NearSettlementOutcome, NearStorageBalance,
    };
    use r402_near::exact::facilitator::{
        NearExactFacilitator, NearFacilitatorOps, decode_signed_delegate, settle_request,
        verify_request_json,
    };
    use r402_near::{
        DEFAULT_FT_TRANSFER_GAS, DEFAULT_MAX_SPONSORED_GAS, EMPTY_CONTRACT_CODE_HASH, ONE_YOCTO,
    };
    use r402_protocol::error::ErrorReason;
    use r402_protocol::network::{ChainId, ChainProvider};
    use r402_protocol::payment::{SettleResponse, VerifyRequest, VerifyResponse};
    use serde_json::{Value, json};

    const SENDER: &str = "alice.testnet";
    const RELAYER: &str = "relayer.testnet";
    const ASSET: &str = "usdc.testnet";
    const PAY_TO: &str = "merchant.testnet";
    const AMOUNT: &str = "1000000";

    #[derive(Clone)]
    struct MockRpc {
        height: Result<u64, ()>,
        access_key: Result<Option<NearAccessKeyView>, ()>,
        accounts: HashMap<String, Option<NearAccountView>>,
        account_err: bool,
        token_err: bool,
        balance: Result<u128, ()>,
        storage: Result<NearStorageBalance, ()>,
        outcome: NearSettlementOutcome,
        chain: String,
    }

    impl Default for MockRpc {
        fn default() -> Self {
            Self {
                height: Ok(1000),
                access_key: Ok(Some(NearAccessKeyView {
                    nonce: 0,
                    permission_kind: NearAccessKeyPermissionKind::FullAccess,
                })),
                accounts: HashMap::new(),
                account_err: false,
                token_err: false,
                balance: Ok(10_000_000),
                storage: Ok(NearStorageBalance::Registered),
                outcome: NearSettlementOutcome {
                    transaction: "FIXTURETX".to_owned(),
                    inner_receipt: NearReceiptStatus::Success {
                        value: String::new(),
                    },
                },
                chain: "near:testnet".to_owned(),
            }
        }
    }

    impl NearRpc for MockRpc {
        async fn current_block_height(&self) -> Result<u64, NearRpcError> {
            self.height
                .map_err(|()| NearRpcError::Rpc("rpc down".to_owned()))
        }

        async fn view_account(
            &self,
            account_id: &str,
        ) -> Result<Option<NearAccountView>, NearRpcError> {
            if self.account_err {
                return Err(NearRpcError::Rpc("account lookup failed".to_owned()));
            }
            if self.token_err && account_id == ASSET {
                return Err(NearRpcError::Rpc("token account lookup failed".to_owned()));
            }
            if let Some(v) = self.accounts.get(account_id) {
                return Ok(v.clone());
            }
            Ok(Some(NearAccountView {
                code_hash: "11111111111111111111111111111112".to_owned(),
            }))
        }

        async fn view_access_key(
            &self,
            _account_id: &str,
            _public_key: &str,
        ) -> Result<Option<NearAccessKeyView>, NearRpcError> {
            self.access_key
                .clone()
                .map_err(|()| NearRpcError::Rpc("access key lookup failed".to_owned()))
        }

        async fn ft_balance_of(
            &self,
            _token: &str,
            _account_id: &str,
        ) -> Result<u128, NearRpcError> {
            self.balance
                .map_err(|()| NearRpcError::Rpc("balance check failed".to_owned()))
        }

        async fn storage_balance_of(
            &self,
            _token: &str,
            _account_id: &str,
        ) -> Result<NearStorageBalance, NearRpcError> {
            self.storage
                .map_err(|()| NearRpcError::Rpc("storage check failed".to_owned()))
        }
    }

    impl ChainProvider for MockRpc {
        fn signer_addresses(&self) -> Vec<String> {
            vec![RELAYER.to_owned()]
        }

        fn chain_id(&self) -> ChainId {
            self.chain.parse().expect("mock CAIP-2")
        }
    }

    impl NearFacilitatorOps for MockRpc {
        fn relayer_ids(&self) -> Vec<String> {
            vec![RELAYER.to_owned()]
        }

        fn max_sponsored_gas(&self) -> u64 {
            DEFAULT_MAX_SPONSORED_GAS
        }

        fn submit_signed_delegate(
            &self,
            _relayer_id: &str,
            _signed_delegate_b64: &str,
        ) -> impl Future<Output = Result<NearSettlementOutcome, NearRpcError>> + Send {
            let outcome = self.outcome.clone();
            async move { Ok(outcome) }
        }
    }

    fn test_signer() -> Signer {
        let account: AccountId = SENDER.parse().unwrap();
        InMemorySigner::from_seed(account, KeyType::ED25519, SENDER)
    }

    struct DelegateOpts {
        sender_id: String,
        receiver_id: String,
        method_name: String,
        ft_receiver: String,
        amount: String,
        deposit: u128,
        gas: u64,
        nonce: u64,
        max_block_height: u64,
        extra_action: bool,
        transfer_kind: bool,
        bad_args: bool,
    }

    impl Default for DelegateOpts {
        fn default() -> Self {
            Self {
                sender_id: SENDER.to_owned(),
                receiver_id: ASSET.to_owned(),
                method_name: "ft_transfer".to_owned(),
                ft_receiver: PAY_TO.to_owned(),
                amount: AMOUNT.to_owned(),
                deposit: ONE_YOCTO,
                gas: DEFAULT_FT_TRANSFER_GAS,
                nonce: 5,
                max_block_height: 1060,
                extra_action: false,
                transfer_kind: false,
                bad_args: false,
            }
        }
    }

    fn signed_delegate_b64(opts: DelegateOpts) -> String {
        let signer = test_signer();
        let sender_id: AccountId = opts.sender_id.parse().unwrap();
        let receiver_id: AccountId = opts.receiver_id.parse().unwrap();
        let mut actions = Vec::new();
        if opts.transfer_kind {
            actions.push(
                NonDelegateAction::try_from(Action::Transfer(TransferAction {
                    deposit: Balance::from_yoctonear(1),
                }))
                .unwrap(),
            );
        } else {
            let args = if opts.bad_args {
                b"not-json".to_vec()
            } else {
                serde_json::to_vec(&json!({
                    "receiver_id": opts.ft_receiver,
                    "amount": opts.amount,
                }))
                .unwrap()
            };
            let fc = FunctionCallAction {
                method_name: opts.method_name,
                args,
                gas: Gas::from_gas(opts.gas),
                deposit: Balance::from_yoctonear(opts.deposit),
            };
            actions.push(NonDelegateAction::try_from(Action::FunctionCall(Box::new(fc))).unwrap());
            if opts.extra_action {
                let fc2 = FunctionCallAction {
                    method_name: "ft_transfer".to_owned(),
                    args: serde_json::to_vec(&json!({
                        "receiver_id": PAY_TO,
                        "amount": AMOUNT,
                    }))
                    .unwrap(),
                    gas: Gas::from_gas(DEFAULT_FT_TRANSFER_GAS),
                    deposit: Balance::from_yoctonear(ONE_YOCTO),
                };
                actions.push(
                    NonDelegateAction::try_from(Action::FunctionCall(Box::new(fc2))).unwrap(),
                );
            }
        }
        let delegate = DelegateAction {
            sender_id,
            receiver_id,
            actions,
            nonce: opts.nonce,
            max_block_height: opts.max_block_height,
            public_key: signer.public_key(),
        };
        let signed = SignedDelegateAction::sign(&signer, delegate);
        let bytes = borsh::to_vec(&signed).unwrap();
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
    }

    fn requirements(overrides: Value) -> Value {
        let mut base = json!({
            "scheme": "exact",
            "network": "near:testnet",
            "asset": ASSET,
            "payTo": PAY_TO,
            "amount": AMOUNT,
            "maxTimeoutSeconds": 60
        });
        if let Some(obj) = overrides.as_object() {
            for (k, v) in obj {
                base[k] = v.clone();
            }
        }
        base
    }

    fn request(signed_b64: &str, accepted: &Value, reqs: &Value) -> Value {
        json!({
            "x402Version": 2,
            "paymentPayload": {
                "x402Version": 2,
                "accepted": accepted,
                "payload": { "signedDelegateAction": signed_b64 }
            },
            "paymentRequirements": reqs
        })
    }

    fn reason(response: &VerifyResponse) -> String {
        match response {
            VerifyResponse::Invalid { reason, .. } => {
                reason.as_ref().map_or("", |r| r.as_str()).to_owned()
            }
            VerifyResponse::Valid { .. } => "valid".to_owned(),
            _ => "other".to_owned(),
        }
    }

    fn relayers() -> Vec<String> {
        vec![RELAYER.to_owned()]
    }

    async fn verify_ok() -> (MockRpc, String, Value, Value) {
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let reqs = requirements(json!({}));
        let rpc = MockRpc::default();
        (rpc, b64, reqs.clone(), reqs)
    }

    #[tokio::test]
    async fn verifies_valid_payload() {
        let (rpc, b64, accepted, reqs) = verify_ok().await;
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &accepted, &reqs),
        )
        .await;
        assert!(
            response.is_valid(),
            "expected valid, got {}",
            reason(&response)
        );
        match response {
            VerifyResponse::Valid { payer, .. } => assert_eq!(payer.as_deref(), Some(SENDER)),
            _ => panic!("expected valid"),
        }
    }

    #[tokio::test]
    async fn rejects_wrong_x402_version() {
        let (rpc, b64, accepted, reqs) = verify_ok().await;
        let mut req = request(&b64, &accepted, &reqs);
        req["paymentPayload"]["x402Version"] = json!(1);
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &req,
        )
        .await;
        assert_eq!(reason(&response), "invalid_x402_version");
    }

    #[tokio::test]
    async fn rejects_unsupported_scheme() {
        let (rpc, b64, _, reqs) = verify_ok().await;
        let mut accepted = reqs.clone();
        accepted["scheme"] = json!("upto");
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &accepted, &reqs),
        )
        .await;
        assert_eq!(reason(&response), "unsupported_scheme");
    }

    #[tokio::test]
    async fn rejects_non_near_network() {
        let (rpc, b64, _, _) = verify_ok().await;
        let reqs = requirements(json!({"network": "eip155:8453"}));
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(reason(&response), "invalid_network");
    }

    #[tokio::test]
    async fn rejects_network_mismatch() {
        let (rpc, b64, _, _) = verify_ok().await;
        let accepted = requirements(json!({"network": "near:testnet"}));
        let reqs = requirements(json!({"network": "near:mainnet"}));
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &accepted, &reqs),
        )
        .await;
        assert_eq!(reason(&response), "invalid_exact_near_network_mismatch");
    }

    #[tokio::test]
    async fn rejects_provider_chain_mismatch() {
        let (rpc, b64, _, _) = verify_ok().await;
        let reqs = requirements(json!({"network": "near:mainnet"}));
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(reason(&response), "invalid_exact_near_network_mismatch");
    }

    #[tokio::test]
    async fn facilitator_verify_rejects_other_near_network() {
        use r402_facilitator::Facilitator;

        let rpc = MockRpc::default();
        let fac = NearExactFacilitator::try_new(rpc).expect("try_new");
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let reqs = requirements(json!({"network": "near:mainnet"}));
        let req = VerifyRequest::from(request(&b64, &reqs, &reqs));
        let response = fac.verify(req).await.expect("verify");
        assert_eq!(reason(&response), "invalid_exact_near_network_mismatch");
    }

    #[tokio::test]
    async fn rejects_asset_mismatch() {
        let (rpc, b64, _, reqs) = verify_ok().await;
        let accepted = requirements(json!({"asset": "other.testnet"}));
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &accepted, &reqs),
        )
        .await;
        assert_eq!(reason(&response), "invalid_exact_near_asset_mismatch");
    }

    #[tokio::test]
    async fn rejects_pay_to_mismatch() {
        let (rpc, b64, _, reqs) = verify_ok().await;
        let accepted = requirements(json!({"payTo": "other.testnet"}));
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &accepted, &reqs),
        )
        .await;
        assert_eq!(reason(&response), "invalid_exact_near_pay_to_mismatch");
    }

    #[tokio::test]
    async fn rejects_amount_mismatch() {
        let (rpc, b64, _, reqs) = verify_ok().await;
        let accepted = requirements(json!({"amount": "2"}));
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &accepted, &reqs),
        )
        .await;
        assert_eq!(reason(&response), "invalid_exact_near_amount_mismatch");
    }

    #[tokio::test]
    async fn rejects_zero_timeout() {
        let (rpc, b64, _, _) = verify_ok().await;
        let reqs = requirements(json!({"maxTimeoutSeconds": 0}));
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(reason(&response), "invalid_exact_near_max_timeout");
    }

    #[tokio::test]
    async fn rejects_payload_shape() {
        let rpc = MockRpc::default();
        let reqs = requirements(json!({}));
        let req = json!({
            "x402Version": 2,
            "paymentPayload": {
                "x402Version": 2,
                "accepted": reqs,
                "payload": {}
            },
            "paymentRequirements": reqs
        });
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &req,
        )
        .await;
        assert_eq!(reason(&response), "invalid_exact_near_payload_shape");
    }

    #[tokio::test]
    async fn rejects_undecodable_delegate() {
        let rpc = MockRpc::default();
        let reqs = requirements(json!({}));
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request("@@not-borsh@@", &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_payload_signed_delegate_action"
        );
    }

    #[tokio::test]
    async fn rejects_invalid_signature_without_payer() {
        let (rpc, b64, accepted, reqs) = verify_ok().await;
        let mut decoded = decode_signed_delegate(&b64).unwrap();
        decoded.signature = near_crypto::Signature::empty(KeyType::ED25519);
        let bytes = borsh::to_vec(&decoded).unwrap();
        let tampered = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&tampered, &accepted, &reqs),
        )
        .await;
        assert_eq!(reason(&response), "invalid_exact_near_payload_signature");
        match response {
            VerifyResponse::Invalid { payer, .. } => assert!(payer.is_none()),
            _ => panic!("expected invalid"),
        }
    }

    #[tokio::test]
    async fn rejects_relayer_as_payer() {
        let (rpc, b64, accepted, reqs) = verify_ok().await;
        let response = verify_request_json(
            &rpc,
            &[SENDER.to_owned()],
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &accepted, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_relayer_cannot_be_payer"
        );
    }

    #[tokio::test]
    async fn rejects_no_relayer_configured() {
        let (rpc, b64, accepted, reqs) = verify_ok().await;
        let response = verify_request_json(
            &rpc,
            &[],
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &accepted, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_no_relayer_configured"
        );
    }

    #[tokio::test]
    async fn rejects_action_count() {
        let rpc = MockRpc::default();
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts {
            extra_action: true,
            ..DelegateOpts::default()
        });
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(reason(&response), "invalid_exact_near_payload_action_count");
    }

    #[tokio::test]
    async fn rejects_action_kind() {
        let rpc = MockRpc::default();
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts {
            transfer_kind: true,
            ..DelegateOpts::default()
        });
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(reason(&response), "invalid_exact_near_payload_action_kind");
    }

    #[tokio::test]
    async fn rejects_method_name() {
        let rpc = MockRpc::default();
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts {
            method_name: "storage_deposit".to_owned(),
            ..DelegateOpts::default()
        });
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(reason(&response), "invalid_exact_near_payload_method_name");
    }

    #[tokio::test]
    async fn rejects_token_contract_mismatch() {
        let rpc = MockRpc::default();
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts {
            receiver_id: "other.testnet".to_owned(),
            ..DelegateOpts::default()
        });
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_payload_token_contract_mismatch"
        );
    }

    #[tokio::test]
    async fn rejects_recipient_mismatch() {
        let rpc = MockRpc::default();
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts {
            ft_receiver: "attacker.testnet".to_owned(),
            ..DelegateOpts::default()
        });
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_payload_recipient_mismatch"
        );
    }

    #[tokio::test]
    async fn rejects_payload_amount_mismatch() {
        let rpc = MockRpc::default();
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts {
            amount: "999999".to_owned(),
            ..DelegateOpts::default()
        });
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_payload_amount_mismatch"
        );
    }

    #[tokio::test]
    async fn rejects_attached_deposit() {
        let rpc = MockRpc::default();
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts {
            deposit: 0,
            ..DelegateOpts::default()
        });
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_payload_attached_deposit"
        );
    }

    #[tokio::test]
    async fn rejects_gas_limit_exceeded() {
        let rpc = MockRpc::default();
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts {
            gas: 200_000_000_000_000,
            ..DelegateOpts::default()
        });
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_payload_gas_limit_exceeded"
        );
    }

    #[tokio::test]
    async fn rejects_expired_delegate() {
        let rpc = MockRpc {
            height: Ok(1000),
            ..MockRpc::default()
        };
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts {
            max_block_height: 500,
            ..DelegateOpts::default()
        });
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_payload_delegate_action_expired"
        );
    }

    #[tokio::test]
    async fn rejects_timeout_window() {
        let rpc = MockRpc {
            height: Ok(1000),
            ..MockRpc::default()
        };
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts {
            max_block_height: 2000,
            ..DelegateOpts::default()
        });
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_payload_delegate_action_timeout_window_exceeds_max_timeout"
        );
    }

    #[tokio::test]
    async fn rejects_nonce_out_of_range() {
        let rpc = MockRpc {
            height: Ok(1000),
            ..MockRpc::default()
        };
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts {
            nonce: 2_000_000_000,
            ..DelegateOpts::default()
        });
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_payload_delegate_action_nonce_out_of_range"
        );
    }

    #[tokio::test]
    async fn rejects_nonce_already_used() {
        let rpc = MockRpc {
            access_key: Ok(Some(NearAccessKeyView {
                nonce: 10,
                permission_kind: NearAccessKeyPermissionKind::FullAccess,
            })),
            ..MockRpc::default()
        };
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_payload_delegate_action_nonce_already_used"
        );
    }

    #[tokio::test]
    async fn rejects_access_key_not_found() {
        let rpc = MockRpc {
            access_key: Ok(None),
            ..MockRpc::default()
        };
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(reason(&response), "invalid_exact_near_access_key_not_found");
    }

    #[tokio::test]
    async fn rejects_function_call_key() {
        let rpc = MockRpc {
            access_key: Ok(Some(NearAccessKeyView {
                nonce: 0,
                permission_kind: NearAccessKeyPermissionKind::FunctionCall,
            })),
            ..MockRpc::default()
        };
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_function_call_key_not_allowed"
        );
    }

    #[tokio::test]
    async fn rejects_unsupported_access_key() {
        let rpc = MockRpc {
            access_key: Ok(Some(NearAccessKeyView {
                nonce: 0,
                permission_kind: NearAccessKeyPermissionKind::Unknown,
            })),
            ..MockRpc::default()
        };
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_unsupported_access_key_permission"
        );
    }

    #[tokio::test]
    async fn rejects_ft_transfer_args() {
        let rpc = MockRpc::default();
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts {
            bad_args: true,
            ..DelegateOpts::default()
        });
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_payload_ft_transfer_args"
        );
    }

    #[tokio::test]
    async fn rejects_account_lookup_failed() {
        let rpc = MockRpc {
            account_err: true,
            ..MockRpc::default()
        };
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_account_lookup_failed"
        );
    }

    #[tokio::test]
    async fn rejects_sender_account_not_found() {
        let mut rpc = MockRpc::default();
        rpc.accounts.insert(SENDER.to_owned(), None);
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_sender_account_not_found"
        );
    }

    #[tokio::test]
    async fn rejects_token_account_lookup_failed() {
        let rpc = MockRpc {
            token_err: true,
            ..MockRpc::default()
        };
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_token_account_lookup_failed"
        );
    }

    #[tokio::test]
    async fn rejects_token_account_not_found() {
        let mut rpc = MockRpc::default();
        rpc.accounts.insert(ASSET.to_owned(), None);
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_token_account_not_found"
        );
    }

    #[tokio::test]
    async fn rejects_token_contract_no_code() {
        let mut rpc = MockRpc::default();
        rpc.accounts.insert(
            ASSET.to_owned(),
            Some(NearAccountView {
                code_hash: EMPTY_CONTRACT_CODE_HASH.to_owned(),
            }),
        );
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_token_contract_no_code"
        );
    }

    #[tokio::test]
    async fn rejects_insufficient_funds() {
        let rpc = MockRpc {
            balance: Ok(1),
            ..MockRpc::default()
        };
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(reason(&response), "insufficient_funds");
    }

    #[tokio::test]
    async fn rejects_unregistered_storage() {
        let rpc = MockRpc {
            storage: Ok(NearStorageBalance::Unregistered),
            ..MockRpc::default()
        };
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_recipient_not_registered_for_storage"
        );
    }

    #[tokio::test]
    async fn allows_unsupported_nep145() {
        let rpc = MockRpc {
            storage: Ok(NearStorageBalance::Unsupported),
            ..MockRpc::default()
        };
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert!(response.is_valid());
    }

    #[tokio::test]
    async fn fails_closed_on_block_height_error() {
        let rpc = MockRpc {
            height: Err(()),
            ..MockRpc::default()
        };
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_current_block_height_unavailable"
        );
    }

    #[tokio::test]
    async fn fails_closed_on_access_key_lookup() {
        let rpc = MockRpc {
            access_key: Err(()),
            ..MockRpc::default()
        };
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(
            reason(&response),
            "invalid_exact_near_access_key_lookup_failed"
        );
    }

    #[tokio::test]
    async fn fails_closed_on_balance_check() {
        let rpc = MockRpc {
            balance: Err(()),
            ..MockRpc::default()
        };
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(reason(&response), "invalid_exact_near_balance_check_failed");
    }

    #[tokio::test]
    async fn fails_closed_on_storage_check() {
        let rpc = MockRpc {
            storage: Err(()),
            ..MockRpc::default()
        };
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let response = verify_request_json(
            &rpc,
            &relayers(),
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &request(&b64, &reqs, &reqs),
        )
        .await;
        assert_eq!(reason(&response), "invalid_exact_near_storage_check_failed");
    }

    #[tokio::test]
    async fn borsh_roundtrip_golden() {
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let decoded = decode_signed_delegate(&b64).unwrap();
        assert!(decoded.verify());
        assert_eq!(decoded.delegate_action.sender_id.as_str(), SENDER);
        assert_eq!(decoded.delegate_action.actions.len(), 1);
    }

    #[test]
    fn decodes_and_verifies_ts_signed_delegate_golden() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/near/ts_signed_delegate.json"
        ))
        .expect("fixture json");
        let b64 = fixture
            .get("signedDelegateAction")
            .and_then(Value::as_str)
            .expect("signedDelegateAction");
        let decoded = decode_signed_delegate(b64).expect("ts borsh decode");
        assert!(
            decoded.verify(),
            "TS @near-js SignedDelegate signature must verify under near-primitives"
        );
        assert_eq!(decoded.delegate_action.sender_id.as_str(), "alice.testnet");
        assert_eq!(decoded.delegate_action.receiver_id.as_str(), "usdc.testnet");
        assert_eq!(decoded.delegate_action.nonce, 7);
        assert_eq!(decoded.delegate_action.max_block_height, 9999);
        assert_eq!(decoded.delegate_action.actions.len(), 1);
        assert_eq!(
            decoded.delegate_action.public_key.to_string(),
            fixture
                .get("publicKey")
                .and_then(Value::as_str)
                .expect("publicKey")
        );
    }

    #[tokio::test]
    async fn settle_succeeds_when_inner_receipt_ok() {
        let rpc = MockRpc::default();
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let req = request(&b64, &reqs, &reqs);
        let cache = SettlementCache::new();
        let outcome = rpc.outcome.clone();
        let response = settle_request(
            &rpc,
            &relayers(),
            &cache,
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &req,
            |_relayer, _b64| async move { Ok(outcome) },
        )
        .await;
        assert!(response.is_success());
    }

    #[tokio::test]
    async fn settle_fails_when_inner_receipt_fails() {
        let rpc = MockRpc {
            outcome: NearSettlementOutcome {
                transaction: "TX2".to_owned(),
                inner_receipt: NearReceiptStatus::Failure {
                    error: "NotEnoughBalance".to_owned(),
                },
            },
            ..MockRpc::default()
        };
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let req = request(&b64, &reqs, &reqs);
        let cache = SettlementCache::new();
        let outcome = rpc.outcome.clone();
        let response = settle_request(
            &rpc,
            &relayers(),
            &cache,
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &req,
            |_relayer, _b64| async move { Ok(outcome) },
        )
        .await;
        match response {
            SettleResponse::Failure {
                reason, message, ..
            } => {
                assert_eq!(reason.as_str(), "settlement_failed");
                assert_eq!(message.as_deref(), Some("NotEnoughBalance"));
            }
            _ => panic!("expected failure"),
        }
    }

    #[tokio::test]
    async fn settle_rejects_duplicate() {
        let rpc = MockRpc::default();
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let req = request(&b64, &reqs, &reqs);
        let cache = SettlementCache::new();
        assert_eq!(cache.reserve(&b64), r402_facilitator::Duplicate::No);
        let response = settle_request(
            &rpc,
            &relayers(),
            &cache,
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &req,
            |_relayer, _b64| async move {
                panic!("must not submit duplicate");
            },
        )
        .await;
        match response {
            SettleResponse::Failure { reason, .. } => {
                assert_eq!(reason, ErrorReason::DuplicateSettlement);
            }
            _ => panic!("expected duplicate"),
        }
    }

    #[tokio::test]
    async fn settle_does_not_submit_when_verify_fails() {
        let rpc = MockRpc::default();
        let reqs = requirements(json!({}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let mut decoded = decode_signed_delegate(&b64).unwrap();
        decoded.signature = near_crypto::Signature::empty(KeyType::ED25519);
        let bytes = borsh::to_vec(&decoded).unwrap();
        let tampered = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
        let req = request(&tampered, &reqs, &reqs);
        let cache = SettlementCache::new();
        let response = settle_request(
            &rpc,
            &relayers(),
            &cache,
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &req,
            |_relayer, _b64| async move {
                panic!("must not submit invalid payload");
            },
        )
        .await;
        assert!(!response.is_success());
    }

    #[tokio::test]
    async fn settle_does_not_submit_when_provider_chain_mismatches() {
        let rpc = MockRpc::default();
        let reqs = requirements(json!({"network": "near:mainnet"}));
        let b64 = signed_delegate_b64(DelegateOpts::default());
        let req = request(&b64, &reqs, &reqs);
        let cache = SettlementCache::new();
        let response = settle_request(
            &rpc,
            &relayers(),
            &cache,
            DEFAULT_MAX_SPONSORED_GAS,
            "near:testnet",
            &req,
            |_relayer, _b64| async move {
                panic!("must not submit cross-network payload");
            },
        )
        .await;
        match response {
            SettleResponse::Failure {
                reason, network, ..
            } => {
                assert_eq!(reason.as_str(), "invalid_exact_near_network_mismatch");
                assert_eq!(network, "near:mainnet");
            }
            _ => panic!("expected failure"),
        }
    }
}
