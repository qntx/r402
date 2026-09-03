//! In-process auth-capture tests. No live RPC. No HTTP E2E.

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
use r402_evm::auth_capture::payload::{
    AUTH_CAPTURE_ESCROW_V1_0_ADDRESS, AuthCaptureExtra, AuthCapturePayload,
    EIP3009_TOKEN_COLLECTOR_ADDRESS, EIP3009_TOKEN_COLLECTOR_V1_0_ADDRESS,
};
use r402_evm::chain::{ChecksummedAddress, Eip155ChainProvider, Eip155ChainReference};
use r402_evm::{Eip155AuthCapture, Eip155AuthCaptureClient, Eip155AuthCaptureFacilitator, USDC};
use r402_facilitator::Facilitator;
use r402_protocol::error::{AsPaymentProblem, FacilitatorError, VerificationError};
use r402_protocol::payment::{PaymentRequired, ResourceInfo, UnixTimestamp, VerifyRequest};
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

fn sample_extra() -> AuthCaptureExtra {
    let now = UnixTimestamp::now().as_secs();
    AuthCaptureExtra {
        name: "USD Coin".into(),
        version: "2".into(),
        capture_authorizer: ChecksummedAddress(Address::repeat_byte(0x11)),
        capture_deadline: now + 3600,
        refund_deadline: now + 86400,
        fee_recipient: ChecksummedAddress(Address::repeat_byte(0x22)),
        min_fee_bps: 0,
        max_fee_bps: 1000,
        auto_capture: Some(false),
        asset_transfer_method: None,
        auth_capture_escrow: None,
    }
}

#[test]
fn price_tag_embeds_auth_capture_extra() {
    let extra = sample_extra();
    let tag =
        Eip155AuthCapture::price_tag(pay_to(), USDC::base().amount(1_000_000u64), extra.clone());
    assert_eq!(tag.requirements.scheme, "auth-capture");
    assert_eq!(tag.requirements.network.to_string(), "eip155:8453");
    let decoded: AuthCaptureExtra =
        serde_json::from_value(tag.requirements.extra.unwrap()).unwrap();
    assert_eq!(decoded.name, extra.name);
    assert_eq!(decoded.capture_deadline, extra.capture_deadline);
    assert!(decoded.asset_transfer_method.is_none());
}

#[test]
fn auth_capture_scheme_id_and_payment_flows() {
    let scheme = Eip155AuthCapture;
    assert_eq!(scheme.namespace(), "eip155");
    assert_eq!(SchemeNetworkServer::scheme(&scheme), "auth-capture");
    assert_eq!(scheme.default_asset_transfer_method(), "eip3009");
    let expected = r402_server::PaymentFlowConfig::authorization_only();
    assert_eq!(scheme.payment_flows().get("eip3009"), Some(&expected));
    assert_eq!(scheme.payment_flows().get("permit2"), Some(&expected));
    assert_eq!(expected.default, PaymentFlowName::Authorization);
}

fn try_new_question_mark() -> Result<(), FacilitatorError> {
    let _fac = Eip155AuthCaptureFacilitator::try_new(dummy_provider())?;
    Ok(())
}

#[test]
fn try_new_question_mark_compiles() {
    try_new_question_mark().expect("try_new is currently infallible");
}

#[tokio::test]
async fn try_new_supported_is_auth_capture_on_provider_chain() {
    let fac = Eip155AuthCaptureFacilitator::try_new(dummy_provider()).expect("try_new");
    let supported = fac.supported().await.expect("supported");
    let kind = supported.kinds.first().expect("one kind");
    assert_eq!(kind.scheme, "auth-capture");
    assert_eq!(kind.network, "eip155:8453");
    assert_eq!(kind.x402_version, 2);
    assert!(
        !supported.signers.is_empty(),
        "try_new provider must advertise signers"
    );
}

#[tokio::test]
async fn verify_malformed_payload_is_invalid_format() {
    let fac = Eip155AuthCaptureFacilitator::try_new(dummy_provider()).expect("try_new");
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
    let (signer, _) = wallet();
    let client = Eip155AuthCaptureClient::new(Arc::new(signer));
    let tag =
        Eip155AuthCapture::price_tag(pay_to(), USDC::base().amount(1_000_000u64), sample_extra());
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()]);
    let candidates = client.accept(&required);
    assert_eq!(candidates.len(), 1, "one auth-capture accept");
    let b64 = candidates
        .first()
        .expect("one accept")
        .sign()
        .await
        .expect("local EIP-712 sign");
    let json = r402_protocol::payment::Base64Bytes(b64.into_bytes())
        .decode()
        .expect("b64");
    let payload: r402_evm::auth_capture::payload::v2::PaymentPayload =
        serde_json::from_slice(&json).expect("payload");
    assert!(matches!(payload.payload, AuthCapturePayload::Eip3009(_)));
    assert_eq!(payload.accepted.network.to_string(), "eip155:8453");
}

#[tokio::test]
async fn facilitator_verify_signed_eip3009() {
    let (signer, _) = wallet();
    let client = Eip155AuthCaptureClient::new(Arc::new(signer));
    let tag =
        Eip155AuthCapture::price_tag(pay_to(), USDC::base().amount(1_000_000u64), sample_extra());
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()]);
    let b64 = client
        .accept(&required)
        .first()
        .expect("one accept")
        .sign()
        .await
        .expect("sign");
    let json = r402_protocol::payment::Base64Bytes(b64.into_bytes())
        .decode()
        .expect("b64");
    let payload: r402_evm::auth_capture::payload::v2::PaymentPayload =
        serde_json::from_slice(&json).expect("payload");
    let typed = r402_evm::auth_capture::payload::v2::VerifyRequest {
        x402_version: r402_protocol::payment::V2,
        payment_payload: payload,
        payment_requirements: serde_json::from_value(
            serde_json::to_value(&tag.requirements).unwrap(),
        )
        .unwrap(),
    };
    let request = VerifyRequest::try_from(typed).expect("typed verify");
    let fac = Eip155AuthCaptureFacilitator::try_new(dummy_provider()).expect("try_new");
    let response = fac.verify(request).await.expect("verify");
    assert!(response.is_valid());
}

fn verify_reason(err: FacilitatorError) -> String {
    match err {
        FacilitatorError::Verification(e) => e.as_payment_problem().reason().as_str().to_owned(),
        other => other.to_string(),
    }
}

async fn signed_eip3009(
    extra: AuthCaptureExtra,
) -> r402_evm::auth_capture::payload::v2::PaymentPayload {
    let (signer, _) = wallet();
    let client = Eip155AuthCaptureClient::new(Arc::new(signer));
    let tag = Eip155AuthCapture::price_tag(pay_to(), USDC::base().amount(1_000_000u64), extra);
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()]);
    let b64 = client
        .accept(&required)
        .first()
        .expect("one accept")
        .sign()
        .await
        .expect("sign");
    let json = r402_protocol::payment::Base64Bytes(b64.into_bytes())
        .decode()
        .expect("b64");
    serde_json::from_slice(&json).expect("payload")
}

#[tokio::test]
async fn default_client_eip3009_to_is_v1_1_collector() {
    let payload = signed_eip3009(sample_extra()).await;
    let AuthCapturePayload::Eip3009(p) = payload.payload else {
        panic!("default extra is eip3009");
    };
    assert_eq!(p.authorization.to, EIP3009_TOKEN_COLLECTOR_ADDRESS);
    assert_eq!(
        p.authorization.to.to_checksum(None),
        "0xEA902B37036bcb4944577ec2101ABdEDF56EbD28"
    );
}

#[tokio::test]
async fn v1_0_escrow_extra_selects_v1_0_collector() {
    let mut extra = sample_extra();
    extra.auth_capture_escrow = Some(ChecksummedAddress(AUTH_CAPTURE_ESCROW_V1_0_ADDRESS));
    let payload = signed_eip3009(extra).await;
    let AuthCapturePayload::Eip3009(p) = payload.payload else {
        panic!("eip3009");
    };
    assert_eq!(p.authorization.to, EIP3009_TOKEN_COLLECTOR_V1_0_ADDRESS);
    assert_eq!(
        p.authorization.to.to_checksum(None),
        "0x0E3dF9510de65469C4518D7843919c0b8C7A7757"
    );
}

#[tokio::test]
async fn unknown_escrow_verify_is_invalid_auth_capture_evm_extra() {
    let payload = signed_eip3009(sample_extra()).await;
    let mut extra = sample_extra();
    extra.auth_capture_escrow = Some(ChecksummedAddress(Address::repeat_byte(0x99)));
    let tag = Eip155AuthCapture::price_tag(pay_to(), USDC::base().amount(1_000_000u64), extra);
    let typed = r402_evm::auth_capture::payload::v2::VerifyRequest {
        x402_version: r402_protocol::payment::V2,
        payment_payload: payload,
        payment_requirements: serde_json::from_value(
            serde_json::to_value(&tag.requirements).unwrap(),
        )
        .unwrap(),
    };
    let request = VerifyRequest::try_from(typed).expect("typed verify");
    let fac = Eip155AuthCaptureFacilitator::try_new(dummy_provider()).expect("try_new");
    let err = fac.verify(request).await.expect_err("unknown escrow");
    assert_eq!(verify_reason(err), "invalid_auth_capture_evm_extra");
}

#[tokio::test]
async fn mixed_fee_fields_are_payload_format() {
    let mut payload = signed_eip3009(sample_extra()).await;
    match &mut payload.payload {
        AuthCapturePayload::Eip3009(p) => {
            p.fee_bps = Some(100);
            p.fee_amount = Some(r402_evm::chain::TokenAmount::from(
                alloy_primitives::U256::from(1_u64),
            ));
        }
        AuthCapturePayload::Permit2(_) => panic!("eip3009"),
    }
    let tag =
        Eip155AuthCapture::price_tag(pay_to(), USDC::base().amount(1_000_000u64), sample_extra());
    let typed = r402_evm::auth_capture::payload::v2::VerifyRequest {
        x402_version: r402_protocol::payment::V2,
        payment_payload: payload,
        payment_requirements: serde_json::from_value(
            serde_json::to_value(&tag.requirements).unwrap(),
        )
        .unwrap(),
    };
    let request = VerifyRequest::try_from(typed).expect("typed verify");
    let fac = Eip155AuthCaptureFacilitator::try_new(dummy_provider()).expect("try_new");
    let err = fac.verify(request).await.expect_err("mixed fee");
    assert_eq!(
        verify_reason(err),
        "invalid_auth_capture_evm_payload_format"
    );
}

#[tokio::test]
async fn v1_1_rejects_fee_bps_pin() {
    let mut payload = signed_eip3009(sample_extra()).await;
    match &mut payload.payload {
        AuthCapturePayload::Eip3009(p) => p.fee_bps = Some(100),
        AuthCapturePayload::Permit2(_) => panic!("eip3009"),
    }
    let tag =
        Eip155AuthCapture::price_tag(pay_to(), USDC::base().amount(1_000_000u64), sample_extra());
    let typed = r402_evm::auth_capture::payload::v2::VerifyRequest {
        x402_version: r402_protocol::payment::V2,
        payment_payload: payload,
        payment_requirements: serde_json::from_value(
            serde_json::to_value(&tag.requirements).unwrap(),
        )
        .unwrap(),
    };
    let request = VerifyRequest::try_from(typed).expect("typed verify");
    let fac = Eip155AuthCaptureFacilitator::try_new(dummy_provider()).expect("try_new");
    let err = fac.verify(request).await.expect_err("feeBps on v1.1");
    assert_eq!(
        verify_reason(err),
        "invalid_auth_capture_evm_payload_format"
    );
}

#[tokio::test]
async fn unknown_escrow_client_refuses_to_sign() {
    let (signer, _) = wallet();
    let client = Eip155AuthCaptureClient::new(Arc::new(signer));
    let mut extra = sample_extra();
    extra.auth_capture_escrow = Some(ChecksummedAddress(Address::repeat_byte(0x99)));
    let tag = Eip155AuthCapture::price_tag(pay_to(), USDC::base().amount(1_000_000u64), extra);
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()]);
    let err = client
        .accept(&required)
        .first()
        .expect("one accept")
        .sign()
        .await
        .expect_err("unknown escrow");
    assert!(err.to_string().contains("authCaptureEscrow"), "got {err}");
}
