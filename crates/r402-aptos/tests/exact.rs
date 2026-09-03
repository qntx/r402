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
    clippy::field_reassign_with_default,
    clippy::needless_pass_by_value,
    reason = "idiomatic test-code patterns"
)]

//! In-process exact-scheme tests. No live fullnode. No HTTP E2E.

use r402_aptos::chain::AptosChainReference;
use r402_aptos::{APTOS_NETWORKS, AptosExact, USDC, USDC_MAINNET_FA};
use r402_protocol::scheme::SchemeId;

#[test]
fn crate_name_matches_directory() {
    assert_eq!(env!("CARGO_PKG_NAME"), "r402-aptos");
}

#[test]
fn aptos_exact_scheme_id() {
    assert_eq!(AptosExact.namespace(), "aptos");
    assert_eq!(AptosExact.scheme(), "exact");
    assert_eq!(AptosExact.caip_family(), "aptos:*");
}

#[test]
fn networks_table_has_mainnet_and_testnet() {
    assert_eq!(APTOS_NETWORKS.len(), 2);
    let names: Vec<_> = APTOS_NETWORKS.iter().map(|n| n.name).collect();
    assert_eq!(names, ["aptos", "aptos-testnet"]);
}

#[test]
fn default_usdc_fa_addresses_match_oracle() {
    assert_eq!(USDC::all().len(), 2);
    assert_eq!(USDC::aptos().address.as_str(), USDC_MAINNET_FA);
    assert_eq!(
        USDC::aptos_testnet().address.as_str(),
        r402_aptos::chain::USDC_TESTNET_FA
    );
}

#[test]
fn chain_reference_converts_to_caip2() {
    let chain: r402_protocol::ChainId = AptosChainReference::TESTNET.into();
    assert_eq!(chain.to_string(), "aptos:2");
}

#[test]
fn extra_fixture_round_trips() {
    let json = include_str!("../../../tests/fixtures/aptos/extra.json");
    let extra: r402_aptos::exact::AptosExtra = serde_json::from_str(json).unwrap();
    assert_eq!(
        extra
            .fee_payer
            .as_ref()
            .map(r402_aptos::chain::AptosAddress::as_str),
        Some("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
}

#[cfg(feature = "client")]
#[test]
fn find_default_aptos_asset_covers_usdc() {
    use r402_aptos::find_default_aptos_asset;
    use r402_protocol::ChainId;

    let mainnet: ChainId = "aptos:1".parse().expect("mainnet");
    let usdc = find_default_aptos_asset(USDC_MAINNET_FA, &mainnet).expect("USDC");
    assert_eq!(usdc.symbol, "USDC");
    assert_eq!(usdc.decimals, 6);
    assert!(find_default_aptos_asset("0x1", &mainnet).is_none());
}

#[cfg(feature = "server")]
#[test]
fn price_tag_is_two_arg() {
    use r402_aptos::chain::AptosAddress;

    let pay_to: AptosAddress = "0x0000000000000000000000000000000000000000000000000000000000000001"
        .parse()
        .unwrap();
    let tag = AptosExact::price_tag(pay_to, USDC::aptos_testnet().amount(1_000_000u64));
    assert_eq!(tag.requirements.scheme, "exact");
    assert_eq!(tag.requirements.network.to_string(), "aptos:2");
    assert!(tag.requirements.extra.is_none());
}

#[cfg(feature = "facilitator")]
fn try_new_question_mark(
    provider: r402_aptos::AptosChainProvider,
) -> Result<(), r402_protocol::FacilitatorError> {
    let _fac = r402_aptos::AptosExactFacilitator::try_new(provider)?;
    Ok(())
}

#[cfg(feature = "facilitator")]
#[test]
fn try_new_question_mark_compiles() {
    use aptos_sdk::account::Ed25519Account;
    use r402_aptos::AptosChainProvider;
    use r402_aptos::chain::AptosFeePayer;

    let payer = AptosFeePayer::new(Ed25519Account::generate());
    let provider = AptosChainProvider::new(
        AptosChainReference::TESTNET,
        vec![payer],
        Some("https://example.invalid"),
    )
    .expect("provider constructs without connecting");
    try_new_question_mark(provider).expect("try_new is currently infallible");
}

#[cfg(feature = "facilitator")]
#[tokio::test]
async fn try_new_supported_is_exact_on_provider_chain() {
    use aptos_sdk::account::Ed25519Account;
    use r402_aptos::chain::{AptosChainProvider, AptosFeePayer};
    use r402_aptos::exact::AptosExactFacilitator;
    use r402_facilitator::Facilitator;

    let account = Ed25519Account::generate();
    let fee_payer = account.address().to_long_string();
    let provider = AptosChainProvider::new(
        AptosChainReference::TESTNET,
        vec![AptosFeePayer::new(account)],
        Some("https://example.invalid"),
    )
    .expect("provider constructs without connecting");
    let fac = AptosExactFacilitator::try_new(provider).expect("try_new");
    let supported = fac.supported().await.expect("supported");
    let kind = supported.kinds.first().expect("one kind");
    assert_eq!(kind.scheme, "exact");
    assert_eq!(kind.network, "aptos:2");
    assert_eq!(kind.x402_version, 2);
    assert!(
        !supported.signers.is_empty(),
        "try_new provider must advertise signers"
    );
    let extra = kind.extra.as_ref().expect("feePayer extra");
    assert_eq!(extra["feePayer"], fee_payer);
}

#[cfg(feature = "facilitator")]
mod verify_settle {
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use aptos_sdk::account::{Ed25519Account, MultiEd25519Account};
    use aptos_sdk::crypto::Ed25519PrivateKey;
    use aptos_sdk::transaction::authenticator::{AccountAuthenticator, Ed25519Signature};
    use aptos_sdk::transaction::builder::TransactionBuilder;
    use aptos_sdk::transaction::{EntryFunction, InputEntryFunctionData};
    use aptos_sdk::types::{AccountAddress, ChainId as AptosSdkChainId};
    use r402_aptos::chain::codec::{
        SimpleTransaction, account_authenticator_for, encode_aptos_payload,
    };
    use r402_aptos::chain::{AptosProviderError, AptosSimulationResult, USDC_TESTNET_FA};
    use r402_aptos::exact::facilitator::{
        AptosFacilitatorOps, settle_request, verify_request_json,
    };
    use r402_aptos::{DEFAULT_CLIENT_MAX_GAS, MAX_GAS_AMOUNT, MAX_GAS_UNIT_PRICE};
    use r402_facilitator::SettlementCache;
    use r402_protocol::error::ErrorReason;
    use r402_protocol::network::{ChainId, ChainProvider};
    use r402_protocol::payment::{SettleResponse, VerifyResponse};
    use serde_json::{Value, json};

    const PAY_TO: &str = "0x0000000000000000000000000000000000000000000000000000000000000001";
    const AMOUNT: &str = "1000";

    #[derive(Clone)]
    struct MockProvider {
        fee_payers: Vec<String>,
        balance: u64,
        simulation: AptosSimulationResult,
        balance_err: Option<String>,
        simulate_err: Option<String>,
        submit: Result<String, String>,
        simulate_calls: Arc<Mutex<u32>>,
    }

    impl Default for MockProvider {
        fn default() -> Self {
            Self {
                fee_payers: Vec::new(),
                balance: 1_000_000,
                simulation: AptosSimulationResult {
                    success: true,
                    vm_status: "Executed successfully".to_owned(),
                },
                balance_err: None,
                simulate_err: None,
                submit: Ok(
                    "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
                ),
                simulate_calls: Arc::new(Mutex::new(0)),
            }
        }
    }

    impl ChainProvider for MockProvider {
        fn signer_addresses(&self) -> Vec<String> {
            self.fee_payers.clone()
        }

        fn chain_id(&self) -> ChainId {
            ChainId::new("aptos", "2")
        }
    }

    impl AptosFacilitatorOps for MockProvider {
        fn fee_payer_addresses(&self) -> Vec<String> {
            self.fee_payers.clone()
        }

        fn sponsor_transactions(&self) -> bool {
            true
        }

        async fn fungible_asset_balance(
            &self,
            _owner: &str,
            _asset: &str,
            _network: &str,
        ) -> Result<u64, AptosProviderError> {
            if let Some(err) = &self.balance_err {
                return Err(AptosProviderError::Rpc(err.clone()));
            }
            Ok(self.balance)
        }

        async fn simulate_payment(
            &self,
            _transaction_base64: &str,
            _network: &str,
        ) -> Result<AptosSimulationResult, AptosProviderError> {
            *self.simulate_calls.lock().unwrap() += 1;
            if let Some(err) = &self.simulate_err {
                return Err(AptosProviderError::Rpc(err.clone()));
            }
            Ok(self.simulation.clone())
        }

        async fn sign_and_submit(
            &self,
            _transaction_base64: &str,
            _fee_payer: Option<&str>,
            _network: &str,
        ) -> Result<String, AptosProviderError> {
            self.submit.clone().map_err(AptosProviderError::Submit)
        }
    }

    struct SignedSample {
        sender: Ed25519Account,
        fee_payer: Ed25519Account,
        encoded: String,
    }

    fn future_expiration() -> u64 {
        now() + 3600
    }

    fn past_expiration() -> u64 {
        now().saturating_sub(100)
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn signed_sample(opts: SampleOpts) -> SignedSample {
        let sender = Ed25519Account::generate();
        let fee_payer = Ed25519Account::generate();
        let pay_to = AccountAddress::from_hex(opts.pay_to.unwrap_or(PAY_TO)).unwrap();
        let asset = AccountAddress::from_hex(opts.asset.unwrap_or(USDC_TESTNET_FA)).unwrap();
        let amount: u64 = opts.amount.unwrap_or(AMOUNT).parse().unwrap();
        let payload = match opts.payload {
            SamplePayload::DefaultTransfer => {
                InputEntryFunctionData::transfer_fungible_asset(asset, pay_to, amount).unwrap()
            }
            SamplePayload::AptTransfer => {
                EntryFunction::apt_transfer(pay_to, amount).unwrap().into()
            }
            SamplePayload::MissingTypeArg => {
                InputEntryFunctionData::new("0x1::primary_fungible_store::transfer")
                    .arg(asset)
                    .arg(pay_to)
                    .arg(amount)
                    .build()
                    .unwrap()
            }
            SamplePayload::MissingAmountArg => {
                InputEntryFunctionData::new("0x1::primary_fungible_store::transfer")
                    .type_arg("0x1::fungible_asset::Metadata")
                    .arg(asset)
                    .arg(pay_to)
                    .build()
                    .unwrap()
            }
        };
        let raw = TransactionBuilder::new()
            .sender(sender.address())
            .sequence_number(0)
            .payload(payload)
            .max_gas_amount(opts.max_gas)
            .gas_unit_price(opts.gas_unit_price)
            .expiration_timestamp_secs(opts.expiration)
            .chain_id(AptosSdkChainId::new(opts.chain_id))
            .build()
            .unwrap();
        let transaction = SimpleTransaction {
            raw_transaction: raw,
            fee_payer_address: opts.sponsored.then_some(fee_payer.address()),
        };
        let message = transaction.signing_message().unwrap();
        let authenticator = account_authenticator_for(&sender, &message).unwrap();
        let mut encoded = encode_aptos_payload(&transaction, &authenticator).unwrap();
        if opts.zero_signature {
            let decoded =
                r402_aptos::chain::codec::DecodedAptosPayment::from_base64(&encoded).unwrap();
            let AccountAuthenticator::Ed25519 { public_key, .. } = decoded.sender_authenticator
            else {
                panic!("expected ed25519 authenticator");
            };
            let forged = AccountAuthenticator::Ed25519 {
                public_key,
                signature: Ed25519Signature([0u8; 64]),
            };
            encoded = encode_aptos_payload(&decoded.transaction, &forged).unwrap();
        }
        SignedSample {
            sender,
            fee_payer,
            encoded,
        }
    }

    struct SampleOpts {
        sponsored: bool,
        chain_id: u8,
        max_gas: u64,
        gas_unit_price: u64,
        expiration: u64,
        amount: Option<&'static str>,
        asset: Option<&'static str>,
        pay_to: Option<&'static str>,
        zero_signature: bool,
        payload: SamplePayload,
    }

    impl Default for SampleOpts {
        fn default() -> Self {
            Self {
                sponsored: true,
                chain_id: 2,
                max_gas: DEFAULT_CLIENT_MAX_GAS,
                gas_unit_price: 100,
                expiration: future_expiration(),
                amount: None,
                asset: None,
                pay_to: None,
                zero_signature: false,
                payload: SamplePayload::DefaultTransfer,
            }
        }
    }

    #[derive(Clone, Copy)]
    enum SamplePayload {
        DefaultTransfer,
        AptTransfer,
        MissingTypeArg,
        MissingAmountArg,
    }

    fn requirements(fee_payer: Option<&str>) -> Value {
        let extra =
            fee_payer.map_or_else(|| json!({}), |fee_payer| json!({ "feePayer": fee_payer }));
        json!({
            "scheme": "exact",
            "network": "aptos:2",
            "asset": USDC_TESTNET_FA,
            "amount": AMOUNT,
            "payTo": PAY_TO,
            "maxTimeoutSeconds": 3600,
            "extra": extra,
        })
    }

    fn request(requirements: &Value, transaction: &str) -> Value {
        json!({
            "x402Version": 2,
            "paymentPayload": {
                "x402Version": 2,
                "accepted": {
                    "scheme": requirements["scheme"],
                    "network": requirements["network"],
                },
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
            VerifyResponse::Invalid { reason, .. } => {
                reason.as_ref().map_or("", |r| r.as_str()).to_owned()
            }
            other => panic!("expected invalid, got {other:?}"),
        }
    }

    async fn verify(provider: &MockProvider, req: &Value) -> VerifyResponse {
        verify_request_json(provider, req).await
    }

    #[tokio::test]
    async fn rejects_gas_unit_price_above_cap() {
        let sample = signed_sample(SampleOpts {
            gas_unit_price: MAX_GAS_UNIT_PRICE + 1,
            ..SampleOpts::default()
        });
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![sample.fee_payer.address().to_long_string()];
        let reqs = requirements(Some(&sample.fee_payer.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &sample.encoded)).await;
        assert!(reason(&result).contains("invalid_exact_aptos_payload_gas_unit_price_too_high"));
        assert!(reason(&result).contains(&(MAX_GAS_UNIT_PRICE + 1).to_string()));
    }

    #[tokio::test]
    async fn gas_unit_price_at_limit_reaches_expiration() {
        let sample = signed_sample(SampleOpts {
            gas_unit_price: MAX_GAS_UNIT_PRICE,
            expiration: past_expiration(),
            ..SampleOpts::default()
        });
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![sample.fee_payer.address().to_long_string()];
        let reqs = requirements(Some(&sample.fee_payer.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &sample.encoded)).await;
        assert!(!reason(&result).contains("gas_unit_price"));
        assert_eq!(
            reason(&result),
            "invalid_exact_aptos_payload_transaction_expired"
        );
    }

    #[tokio::test]
    async fn rejects_max_gas_above_cap() {
        let sample = signed_sample(SampleOpts {
            max_gas: MAX_GAS_AMOUNT + 1,
            ..SampleOpts::default()
        });
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![sample.fee_payer.address().to_long_string()];
        let reqs = requirements(Some(&sample.fee_payer.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &sample.encoded)).await;
        assert!(reason(&result).contains("invalid_exact_aptos_payload_gas_too_high"));
    }

    #[tokio::test]
    async fn non_sponsored_skips_gas_caps() {
        let sample = signed_sample(SampleOpts {
            sponsored: false,
            max_gas: MAX_GAS_AMOUNT + 1,
            gas_unit_price: MAX_GAS_UNIT_PRICE + 1,
            expiration: past_expiration(),
            ..SampleOpts::default()
        });
        let provider = MockProvider::default();
        let reqs = requirements(None);
        let result = verify(&provider, &request(&reqs, &sample.encoded)).await;
        assert!(!reason(&result).contains("gas_unit_price"));
        assert!(!reason(&result).contains("gas_too_high"));
        assert_eq!(
            reason(&result),
            "invalid_exact_aptos_payload_transaction_expired"
        );
    }

    #[tokio::test]
    async fn rejects_forged_signature_before_network() {
        let sample = signed_sample(SampleOpts {
            expiration: past_expiration(),
            zero_signature: true,
            ..SampleOpts::default()
        });
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![sample.fee_payer.address().to_long_string()];
        let reqs = requirements(Some(&sample.fee_payer.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &sample.encoded)).await;
        assert_eq!(
            reason(&result),
            "invalid_exact_aptos_payload_sender_authenticator_invalid_signature"
        );
        assert_eq!(*provider.simulate_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn valid_signature_reaches_expiration() {
        let sample = signed_sample(SampleOpts {
            expiration: past_expiration(),
            ..SampleOpts::default()
        });
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![sample.fee_payer.address().to_long_string()];
        let reqs = requirements(Some(&sample.fee_payer.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &sample.encoded)).await;
        assert_eq!(
            reason(&result),
            "invalid_exact_aptos_payload_transaction_expired"
        );
    }

    #[tokio::test]
    async fn verifies_valid_payload() {
        let sample = signed_sample(SampleOpts::default());
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![sample.fee_payer.address().to_long_string()];
        let reqs = requirements(Some(&sample.fee_payer.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &sample.encoded)).await;
        match result {
            VerifyResponse::Valid { payer, .. } => {
                assert_eq!(
                    payer.as_deref(),
                    Some(sample.sender.address().to_long_string().as_str())
                );
            }
            other => panic!("expected valid, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_chain_id_mismatch() {
        let sample = signed_sample(SampleOpts {
            chain_id: 1,
            ..SampleOpts::default()
        });
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![sample.fee_payer.address().to_long_string()];
        let reqs = requirements(Some(&sample.fee_payer.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &sample.encoded)).await;
        assert!(reason(&result).contains("invalid_exact_aptos_payload_chain_id_mismatch"));
    }

    #[tokio::test]
    async fn rejects_fee_payer_not_managed() {
        let sample = signed_sample(SampleOpts::default());
        let provider = MockProvider::default();
        let reqs = requirements(Some(&sample.fee_payer.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &sample.encoded)).await;
        assert_eq!(reason(&result), "fee_payer_not_managed_by_facilitator");
    }

    #[tokio::test]
    async fn rejects_facilitator_as_sender() {
        let fee_payer = Ed25519Account::generate();
        let pay_to = AccountAddress::from_hex(PAY_TO).unwrap();
        let asset = AccountAddress::from_hex(USDC_TESTNET_FA).unwrap();
        let payload = InputEntryFunctionData::transfer_fungible_asset(asset, pay_to, 1000).unwrap();
        let raw = TransactionBuilder::new()
            .sender(fee_payer.address())
            .sequence_number(0)
            .payload(payload)
            .max_gas_amount(DEFAULT_CLIENT_MAX_GAS)
            .gas_unit_price(100)
            .expiration_timestamp_secs(future_expiration())
            .chain_id(AptosSdkChainId::testnet())
            .build()
            .unwrap();
        let transaction = SimpleTransaction {
            raw_transaction: raw,
            fee_payer_address: Some(fee_payer.address()),
        };
        let message = transaction.signing_message().unwrap();
        let authenticator = account_authenticator_for(&fee_payer, &message).unwrap();
        let encoded = encode_aptos_payload(&transaction, &authenticator).unwrap();
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![fee_payer.address().to_long_string()];
        let reqs = requirements(Some(&fee_payer.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &encoded)).await;
        assert_eq!(
            reason(&result),
            "invalid_exact_aptos_payload_fee_payer_transferring_funds"
        );
    }

    #[tokio::test]
    async fn rejects_insufficient_balance() {
        let sample = signed_sample(SampleOpts::default());
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![sample.fee_payer.address().to_long_string()];
        provider.balance = 1;
        let reqs = requirements(Some(&sample.fee_payer.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &sample.encoded)).await;
        assert_eq!(
            reason(&result),
            "invalid_exact_aptos_payload_insufficient_balance"
        );
    }

    #[tokio::test]
    async fn rejects_simulation_failure() {
        let sample = signed_sample(SampleOpts::default());
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![sample.fee_payer.address().to_long_string()];
        provider.simulation = AptosSimulationResult {
            success: false,
            vm_status: "OUT_OF_GAS".to_owned(),
        };
        let reqs = requirements(Some(&sample.fee_payer.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &sample.encoded)).await;
        assert!(reason(&result).contains("invalid_exact_aptos_payload_simulation_failed"));
        assert!(reason(&result).contains("OUT_OF_GAS"));
    }

    #[tokio::test]
    async fn settles_when_verify_passes() {
        let sample = signed_sample(SampleOpts::default());
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![sample.fee_payer.address().to_long_string()];
        let reqs = requirements(Some(&sample.fee_payer.address().to_long_string()));
        let req = request(&reqs, &sample.encoded);
        let settled = settle_request(&provider, &SettlementCache::new(), &req).await;
        match settled {
            SettleResponse::Success {
                transaction, payer, ..
            } => {
                assert!(transaction.starts_with("0x"));
                assert_eq!(
                    payer.as_deref(),
                    Some(sample.sender.address().to_long_string().as_str())
                );
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_duplicate_settle() {
        let sample = signed_sample(SampleOpts::default());
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![sample.fee_payer.address().to_long_string()];
        let reqs = requirements(Some(&sample.fee_payer.address().to_long_string()));
        let req = request(&reqs, &sample.encoded);
        let cache = SettlementCache::new();
        let first = settle_request(&provider, &cache, &req).await;
        assert!(matches!(first, SettleResponse::Success { .. }));
        let second = settle_request(&provider, &cache, &req).await;
        match second {
            SettleResponse::Failure { reason, .. } => {
                assert_eq!(reason, ErrorReason::DuplicateSettlement);
            }
            other => panic!("expected duplicate, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_unsupported_scheme() {
        let provider = MockProvider::default();
        let mut reqs = requirements(None);
        reqs["scheme"] = json!("something-else");
        let result = verify(&provider, &request(&reqs, "")).await;
        assert_eq!(reason(&result), "unsupported_scheme");
    }

    #[tokio::test]
    async fn rejects_network_mismatch() {
        let provider = MockProvider::default();
        let reqs = requirements(None);
        let mut req = request(&reqs, "");
        req["paymentPayload"]["accepted"]["network"] = json!("aptos:1");
        let result = verify(&provider, &req).await;
        assert_eq!(reason(&result), "network_mismatch");
    }

    #[tokio::test]
    async fn encode_fixture_is_json_of_bytes() {
        let sample = signed_sample(SampleOpts::default());
        let decoded = r402_aptos::chain::decode_aptos_payload(&sample.encoded).unwrap();
        assert!(!decoded.transaction.is_empty());
        assert!(!decoded.sender_authenticator.is_empty());
    }

    #[tokio::test]
    async fn rejects_wrong_function() {
        let sample = signed_sample(SampleOpts {
            payload: SamplePayload::AptTransfer,
            ..SampleOpts::default()
        });
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![sample.fee_payer.address().to_long_string()];
        let reqs = requirements(Some(&sample.fee_payer.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &sample.encoded)).await;
        assert_eq!(
            reason(&result),
            "invalid_exact_aptos_payload_wrong_function"
        );
    }

    #[tokio::test]
    async fn rejects_wrong_type_args() {
        let sample = signed_sample(SampleOpts {
            payload: SamplePayload::MissingTypeArg,
            ..SampleOpts::default()
        });
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![sample.fee_payer.address().to_long_string()];
        let reqs = requirements(Some(&sample.fee_payer.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &sample.encoded)).await;
        assert_eq!(
            reason(&result),
            "invalid_exact_aptos_payload_wrong_type_args"
        );
    }

    #[tokio::test]
    async fn rejects_wrong_args() {
        let sample = signed_sample(SampleOpts {
            payload: SamplePayload::MissingAmountArg,
            ..SampleOpts::default()
        });
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![sample.fee_payer.address().to_long_string()];
        let reqs = requirements(Some(&sample.fee_payer.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &sample.encoded)).await;
        assert_eq!(reason(&result), "invalid_exact_aptos_payload_wrong_args");
    }

    #[tokio::test]
    async fn rejects_asset_mismatch() {
        let other_asset = "0x00000000000000000000000000000000000000000000000000000000000000aa";
        let sample = signed_sample(SampleOpts {
            asset: Some(other_asset),
            ..SampleOpts::default()
        });
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![sample.fee_payer.address().to_long_string()];
        let reqs = requirements(Some(&sample.fee_payer.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &sample.encoded)).await;
        assert_eq!(
            reason(&result),
            "invalid_exact_aptos_payload_asset_mismatch"
        );
    }

    #[tokio::test]
    async fn rejects_amount_mismatch() {
        let sample = signed_sample(SampleOpts {
            amount: Some("999"),
            ..SampleOpts::default()
        });
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![sample.fee_payer.address().to_long_string()];
        let reqs = requirements(Some(&sample.fee_payer.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &sample.encoded)).await;
        assert_eq!(
            reason(&result),
            "invalid_exact_aptos_payload_amount_mismatch"
        );
    }

    #[tokio::test]
    async fn rejects_recipient_mismatch() {
        let other = "0x00000000000000000000000000000000000000000000000000000000000000bb";
        let sample = signed_sample(SampleOpts {
            pay_to: Some(other),
            ..SampleOpts::default()
        });
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![sample.fee_payer.address().to_long_string()];
        let reqs = requirements(Some(&sample.fee_payer.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &sample.encoded)).await;
        assert_eq!(
            reason(&result),
            "invalid_exact_aptos_payload_recipient_mismatch"
        );
    }

    #[tokio::test]
    async fn rejects_fee_payer_mismatch() {
        let sample = signed_sample(SampleOpts::default());
        let other = Ed25519Account::generate();
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![
            sample.fee_payer.address().to_long_string(),
            other.address().to_long_string(),
        ];
        let reqs = requirements(Some(&other.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &sample.encoded)).await;
        assert_eq!(
            reason(&result),
            "invalid_exact_aptos_payload_fee_payer_mismatch"
        );
    }

    #[tokio::test]
    async fn rejects_multied25519_authenticator() {
        let keys = vec![Ed25519PrivateKey::generate(), Ed25519PrivateKey::generate()];
        let account = MultiEd25519Account::new(keys, 1).unwrap();
        let fee_payer = Ed25519Account::generate();
        let pay_to = AccountAddress::from_hex(PAY_TO).unwrap();
        let asset = AccountAddress::from_hex(USDC_TESTNET_FA).unwrap();
        let payload = InputEntryFunctionData::transfer_fungible_asset(asset, pay_to, 1000).unwrap();
        let raw = TransactionBuilder::new()
            .sender(account.address())
            .sequence_number(0)
            .payload(payload)
            .max_gas_amount(DEFAULT_CLIENT_MAX_GAS)
            .gas_unit_price(100)
            .expiration_timestamp_secs(future_expiration())
            .chain_id(AptosSdkChainId::testnet())
            .build()
            .unwrap();
        let transaction = SimpleTransaction {
            raw_transaction: raw,
            fee_payer_address: Some(fee_payer.address()),
        };
        let message = transaction.signing_message().unwrap();
        let signature = account.sign(&message).unwrap();
        let authenticator = AccountAuthenticator::MultiEd25519 {
            public_key: account.public_key().to_bytes(),
            signature: signature.to_bytes(),
        };
        let encoded = encode_aptos_payload(&transaction, &authenticator).unwrap();
        let mut provider = MockProvider::default();
        provider.fee_payers = vec![fee_payer.address().to_long_string()];
        let reqs = requirements(Some(&fee_payer.address().to_long_string()));
        let result = verify(&provider, &request(&reqs, &encoded)).await;
        assert_eq!(
            reason(&result),
            "invalid_exact_aptos_payload_unsupported_authenticator"
        );
    }
}
