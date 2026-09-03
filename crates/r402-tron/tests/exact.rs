//! In-process exact-scheme tests. No live RPC. No HTTP E2E.

#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "idiomatic test-code patterns"
)]

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::U256;
use alloy_signer_local::PrivateKeySigner;
use r402_client::SchemeClient;
use r402_facilitator::Facilitator;
use r402_protocol::error::{FacilitatorError, VerificationError};
use r402_protocol::payment::{PaymentRequired, ResourceInfo, VerifyRequest};
use r402_protocol::scheme::SchemeId;
use r402_server::{PaymentFlowName, SchemeNetworkServer};
use r402_tron::chain::{Address, TronChainProvider, TronChainProviderConfig, TronChainReference};
use r402_tron::exact::payload::{ExactPayload, PaymentRequirementsExtra};
use r402_tron::{AssetTransferMethod, TronExact, TronExactClient, TronExactFacilitator, USDT};
use url::Url;

/// Anvil account 0. Local signing only; never sent to a live chain.
const ANVIL_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

fn signer() -> PrivateKeySigner {
    PrivateKeySigner::from_str(ANVIL_KEY).expect("anvil key")
}

fn dummy_provider() -> TronChainProvider {
    TronChainProvider::new(TronChainProviderConfig {
        chain_reference: TronChainReference::NILE,
        base_url: Url::parse("https://example.invalid").expect("url"),
        signer: signer(),
        fee_limit: 100_000_000,
        confirmation_timeout: Duration::from_secs(1),
        confirmation_poll_interval: Duration::from_millis(50),
    })
}

fn pay_to() -> Address {
    Address::from_base58check("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t").expect("pay_to")
}

#[test]
fn price_tag_is_three_arg_eip3009_default() {
    let deployment = USDT::nile().clone().with_tip712("Tether USD", "1");
    let tag = TronExact::price_tag(pay_to(), &deployment.amount(U256::from(1_000_000u64)), None);
    assert_eq!(tag.requirements.scheme, "exact");
    assert_eq!(tag.requirements.network.to_string(), "tron:0xcd8690dc");
    assert_eq!(tag.requirements.amount, "1000000");
    assert_eq!(
        tag.requirements.pay_to,
        "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"
    );
    let extra: PaymentRequirementsExtra =
        serde_json::from_value(tag.requirements.extra.unwrap()).unwrap();
    assert_eq!(extra.name, "Tether USD");
    assert_eq!(extra.version, "1");
    assert!(extra.asset_transfer_method.is_none());
}

#[test]
fn price_tag_third_arg_selects_permit2() {
    let deployment = USDT::nile().clone().with_tip712("Tether USD", "1");
    let tag = TronExact::price_tag(
        pay_to(),
        &deployment.amount(U256::from(1_000_000u64)),
        Some(AssetTransferMethod::Permit2),
    );
    let extra: PaymentRequirementsExtra =
        serde_json::from_value(tag.requirements.extra.unwrap()).unwrap();
    assert_eq!(
        extra.asset_transfer_method,
        Some(AssetTransferMethod::Permit2)
    );
}

#[test]
fn exact_scheme_id_and_payment_flows() {
    let scheme = TronExact;
    assert_eq!(scheme.namespace(), "tron");
    assert_eq!(SchemeNetworkServer::scheme(&scheme), "exact");
    assert_eq!(scheme.default_asset_transfer_method(), "eip3009");
    let expected = r402_server::PaymentFlowConfig::authorization_and_upfront();
    assert_eq!(scheme.payment_flows().get("eip3009"), Some(&expected));
    assert_eq!(scheme.payment_flows().get("permit2"), Some(&expected));
    assert_eq!(expected.default, PaymentFlowName::Authorization);
}

fn try_new_question_mark() -> Result<(), FacilitatorError> {
    let _fac = TronExactFacilitator::try_new(dummy_provider())?;
    Ok(())
}

#[test]
fn try_new_question_mark_compiles() {
    try_new_question_mark().expect("try_new is currently infallible");
}

#[tokio::test]
async fn try_new_supported_is_exact_on_provider_chain() {
    let fac = TronExactFacilitator::try_new(dummy_provider()).expect("try_new");
    let supported = fac.supported().await.expect("supported");
    let kind = supported.kinds.first().expect("one kind");
    assert_eq!(kind.scheme, "exact");
    assert_eq!(kind.network, "tron:0xcd8690dc");
    assert_eq!(kind.x402_version, 2);
    assert!(
        !supported.signers.is_empty(),
        "try_new provider must advertise signers"
    );
}

#[tokio::test]
async fn verify_malformed_payload_is_invalid_format() {
    let fac = TronExactFacilitator::try_new(dummy_provider()).expect("try_new");
    let err = fac
        .verify(VerifyRequest::from(serde_json::json!({})))
        .await
        .expect_err("empty JSON is not a verify request");
    assert!(
        matches!(
            err,
            FacilitatorError::Verification(VerificationError::InvalidFormat(_))
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn client_accepts_and_signs_eip3009_locally() {
    let client = TronExactClient::new(Arc::new(signer()));
    let deployment = USDT::nile().clone().with_tip712("Tether USD", "1");
    let tag = TronExact::price_tag(pay_to(), &deployment.amount(U256::from(1_000_000u64)), None);
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()]);
    let candidates = client.accept(&required);
    assert_eq!(candidates.len(), 1, "one eip3009 accept");
    let b64 = candidates
        .first()
        .expect("one eip3009 accept")
        .sign()
        .await
        .expect("local TIP-712 sign");
    let json = r402_protocol::payment::Base64Bytes(b64.into_bytes())
        .decode()
        .expect("b64");
    let payload: r402_tron::exact::payload::v2::PaymentPayload =
        serde_json::from_slice(&json).expect("payload");
    assert!(matches!(payload.payload, ExactPayload::Eip3009(_)));
    assert_eq!(payload.accepted.network.to_string(), "tron:0xcd8690dc");
}

#[tokio::test]
async fn client_accepts_and_signs_permit2_locally() {
    let client = TronExactClient::new(Arc::new(signer()));
    let deployment = USDT::nile().clone().with_tip712("Tether USD", "1");
    let tag = TronExact::price_tag(
        pay_to(),
        &deployment.amount(U256::from(1_000_000u64)),
        Some(AssetTransferMethod::Permit2),
    );
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()]);
    let candidates = client.accept(&required);
    assert_eq!(candidates.len(), 1, "one permit2 accept");
    let b64 = candidates
        .first()
        .expect("one permit2 accept")
        .sign()
        .await
        .expect("local Permit2 sign");
    let json = r402_protocol::payment::Base64Bytes(b64.into_bytes())
        .decode()
        .expect("b64");
    let payload: r402_tron::exact::payload::v2::PaymentPayload =
        serde_json::from_slice(&json).expect("payload");
    assert!(matches!(payload.payload, ExactPayload::Permit2(_)));
}

#[test]
fn find_default_asset_mainnet_usdt() {
    let network = "tron:0x2b6653dc".parse().unwrap();
    let info = r402_tron::find_default_tron_asset(&USDT::mainnet().address.to_string(), &network)
        .expect("mainnet USDT");
    assert_eq!(info.symbol, "USDT");
    assert_eq!(info.decimals, 6);
}
