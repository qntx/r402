//! In-process upto-scheme tests. No live RPC. No HTTP E2E.

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

use alloy_network::EthereumWallet;
use alloy_primitives::{Address, address};
use alloy_signer_local::PrivateKeySigner;
use r402_client::SchemeClient;
use r402_evm::chain::{Eip155ChainProvider, Eip155ChainReference};
use r402_evm::upto::payload::UptoPaymentRequirementsExtra;
use r402_evm::{
    Eip155Upto, Eip155UptoClient, Eip155UptoFacilitator, USDC, X402_UPTO_PERMIT2_PROXY,
};
use r402_facilitator::Facilitator;
use r402_protocol::error::{AsPaymentProblem, ErrorReason, FacilitatorError, VerificationError};
use r402_protocol::payment::{PaymentRequired, ResourceInfo, SettleRequest, VerifyRequest};
use r402_protocol::scheme::SchemeId;
use r402_server::{PaymentFlowName, SchemeNetworkServer};
use url::Url;

/// Anvil account 0. Local signing only; never sent to a live chain.
const ANVIL_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

fn wallet() -> (PrivateKeySigner, EthereumWallet) {
    let signer = PrivateKeySigner::from_str(ANVIL_KEY).expect("anvil key");
    let wallet = EthereumWallet::from(signer.clone());
    (signer, wallet)
}

fn dummy_provider() -> Eip155ChainProvider {
    let (_signer, wallet) = wallet();
    let url = Url::parse("https://example.invalid").expect("url");
    Eip155ChainProvider::new(
        Eip155ChainReference::new(8453),
        wallet,
        &[(url, None)],
        true,
        false,
        1,
    )
    .expect("provider constructs without connecting")
}

const fn pay_to() -> Address {
    address!("0x1111111111111111111111111111111111111111")
}

fn facilitator_addr() -> Address {
    let (signer, _) = wallet();
    signer.address()
}

#[test]
fn scheme_id_and_payment_flows() {
    let scheme = Eip155Upto;
    assert_eq!(scheme.namespace(), "eip155");
    assert_eq!(SchemeNetworkServer::scheme(&scheme), "upto");
    assert_eq!(scheme.default_asset_transfer_method(), "permit2");
    let expected = r402_server::PaymentFlowConfig::authorization_only();
    assert_eq!(scheme.payment_flows().get("permit2"), Some(&expected));
    assert_eq!(expected.default, PaymentFlowName::Authorization);
    assert_eq!(expected.supported, vec![PaymentFlowName::Authorization]);
}

#[test]
fn proxy_address_is_canonical() {
    assert_eq!(
        X402_UPTO_PERMIT2_PROXY.to_checksum(None),
        "0x4020A4f3b7b90ccA423B9fabCc0CE57C6C240002"
    );
}

#[test]
fn price_tag_is_permit2_max_without_facilitator() {
    let tag = Eip155Upto::price_tag(pay_to(), USDC::base().amount(5_000_000u64));
    assert_eq!(tag.requirements.scheme, "upto");
    assert_eq!(tag.requirements.network.to_string(), "eip155:8453");
    assert_eq!(tag.requirements.amount, "5000000");
    let extra = tag.requirements.extra.unwrap();
    assert_eq!(
        extra
            .get("assetTransferMethod")
            .and_then(serde_json::Value::as_str),
        Some("permit2")
    );
    assert!(extra.get("facilitatorAddress").is_none());
}

#[test]
fn price_tag_with_facilitator_pins_address() {
    let tag = Eip155Upto::price_tag_with_facilitator(
        pay_to(),
        USDC::base().amount(1_000_000u64),
        facilitator_addr(),
    );
    let extra: UptoPaymentRequirementsExtra =
        serde_json::from_value(tag.requirements.extra.unwrap()).unwrap();
    assert_eq!(extra.facilitator_address.0, facilitator_addr());
}

fn try_new_question_mark() -> Result<(), FacilitatorError> {
    let _fac = Eip155UptoFacilitator::try_new(dummy_provider())?;
    Ok(())
}

#[test]
fn try_new_question_mark_compiles() {
    try_new_question_mark().expect("try_new is currently infallible");
}

#[tokio::test]
async fn try_new_supported_is_upto_on_provider_chain() {
    let fac = Eip155UptoFacilitator::try_new(dummy_provider()).expect("try_new");
    let supported = fac.supported().await.expect("supported");
    let kind = supported.kinds.first().expect("one kind");
    assert_eq!(kind.scheme, "upto");
    assert_eq!(kind.network, "eip155:8453");
    assert_eq!(kind.x402_version, 2);
    assert!(
        !supported.signers.is_empty(),
        "try_new provider must advertise signers"
    );
    let extra = kind.extra.as_ref().expect("upto kind extra");
    assert_eq!(
        extra
            .get("assetTransferMethod")
            .and_then(serde_json::Value::as_str),
        Some("permit2")
    );
    assert!(extra.get("facilitatorAddress").is_some());
}

#[tokio::test]
async fn verify_malformed_payload_is_invalid_format() {
    let fac = Eip155UptoFacilitator::try_new(dummy_provider()).expect("try_new");
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
async fn client_accepts_and_signs_permit2_locally() {
    let (signer, _) = wallet();
    let client = Eip155UptoClient::new(Arc::new(signer));
    let tag = Eip155Upto::price_tag_with_facilitator(
        pay_to(),
        USDC::base().amount(1_000_000u64),
        facilitator_addr(),
    );
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()]);
    let candidates = client.accept(&required);
    assert_eq!(candidates.len(), 1, "one upto accept");
    let b64 = candidates
        .first()
        .expect("one upto accept")
        .sign()
        .await
        .expect("local Permit2 sign");
    let json = r402_protocol::payment::Base64Bytes(b64.into_bytes())
        .decode()
        .expect("b64");
    let payload: r402_evm::upto::payload::v2::PaymentPayload =
        serde_json::from_slice(&json).expect("payload");
    assert_eq!(
        payload.payload.permit2_authorization.spender,
        X402_UPTO_PERMIT2_PROXY
    );
    assert_eq!(
        payload.payload.permit2_authorization.witness.facilitator,
        facilitator_addr()
    );
    assert_eq!(payload.accepted.network.to_string(), "eip155:8453");
}

#[tokio::test]
async fn settle_rejects_actual_above_signed_max() {
    let (signer, _) = wallet();
    let client = Eip155UptoClient::new(Arc::new(signer));
    let tag = Eip155Upto::price_tag_with_facilitator(
        pay_to(),
        USDC::base().amount(5_000_000u64),
        facilitator_addr(),
    );
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()]);
    let b64 = client
        .accept(&required)
        .first()
        .expect("candidate")
        .sign()
        .await
        .expect("sign");
    let json = r402_protocol::payment::Base64Bytes(b64.into_bytes())
        .decode()
        .expect("b64");
    let payload: serde_json::Value = serde_json::from_slice(&json).expect("json");

    let mut requirements = tag.requirements.clone();
    requirements.amount = "6000000".into();
    let settle = serde_json::json!({
        "x402Version": 2,
        "paymentPayload": payload,
        "paymentRequirements": requirements,
    });
    let fac = Eip155UptoFacilitator::try_new(dummy_provider()).expect("try_new");
    let err = fac
        .settle(SettleRequest::from(settle))
        .await
        .expect_err("actual above max");
    assert!(
        matches!(
            err,
            FacilitatorError::Verification(
                VerificationError::SettlementAmountExceedsPermitted { .. }
            )
        ),
        "got {err:?}"
    );
}

#[test]
fn permit2_allowance_required_is_412_reason() {
    let problem = VerificationError::Permit2AllowanceRequired.as_payment_problem();
    assert_eq!(problem.reason(), ErrorReason::Permit2AllowanceRequired);
    assert_eq!(problem.reason().as_str(), "permit2_allowance_required");
}
