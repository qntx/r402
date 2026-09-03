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

use alloy_network::EthereumWallet;
use alloy_primitives::{Address, address};
use alloy_signer_local::PrivateKeySigner;
use r402_client::SchemeClient;
use r402_evm::chain::{Eip155ChainProvider, Eip155ChainReference};
use r402_evm::exact::payload::{ExactPayload, PaymentRequirementsExtra};
use r402_evm::{AssetTransferMethod, Eip155Exact, Eip155ExactClient, Eip155ExactFacilitator, USDC};
use r402_facilitator::Facilitator;
use r402_protocol::error::VerificationError;
use r402_protocol::payment::{PaymentRequired, ResourceInfo, VerifyRequest};
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

#[test]
fn price_tag_is_three_arg_eip3009_default() {
    let tag = Eip155Exact::price_tag(pay_to(), USDC::base().amount(1_000_000u64), None);
    assert_eq!(tag.requirements.scheme, "exact");
    assert_eq!(tag.requirements.network.to_string(), "eip155:8453");
    assert_eq!(tag.requirements.amount, "1000000");
    assert_eq!(
        tag.requirements.pay_to,
        "0x1111111111111111111111111111111111111111"
    );
    let extra: PaymentRequirementsExtra =
        serde_json::from_value(tag.requirements.extra.unwrap()).unwrap();
    assert_eq!(extra.name, "USD Coin");
    assert_eq!(extra.version, "2");
    assert!(extra.asset_transfer_method.is_none());
}

#[test]
fn price_tag_third_arg_selects_permit2() {
    let tag = Eip155Exact::price_tag(
        pay_to(),
        USDC::base().amount(1_000_000u64),
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
    let scheme = Eip155Exact;
    assert_eq!(scheme.namespace(), "eip155");
    assert_eq!(SchemeNetworkServer::scheme(&scheme), "exact");
    assert_eq!(scheme.default_asset_transfer_method(), "eip3009");
    let expected = r402_server::PaymentFlowConfig::authorization_and_upfront();
    assert_eq!(scheme.payment_flows().get("eip3009"), Some(&expected));
    assert_eq!(scheme.payment_flows().get("permit2"), Some(&expected));
    assert_eq!(expected.default, PaymentFlowName::Authorization);
}

#[tokio::test]
async fn try_new_supported_is_exact_on_provider_chain() {
    let fac = Eip155ExactFacilitator::try_new(dummy_provider());
    let supported = fac.supported().await.expect("supported");
    let kind = supported.kinds.first().expect("one kind");
    assert_eq!(kind.scheme, "exact");
    assert_eq!(kind.network, "eip155:8453");
    assert_eq!(kind.x402_version, 2);
    assert!(
        !supported.signers.is_empty(),
        "try_new provider must advertise signers"
    );
}

#[tokio::test]
async fn verify_malformed_payload_is_invalid_format() {
    let fac = Eip155ExactFacilitator::try_new(dummy_provider());
    let err = fac
        .verify(VerifyRequest::from(serde_json::json!({})))
        .await
        .expect_err("empty JSON is not a verify request");
    assert!(
        matches!(
            err,
            r402_protocol::error::FacilitatorError::Verification(VerificationError::InvalidFormat(
                _
            ))
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn client_accepts_and_signs_eip3009_locally() {
    let (signer, _) = wallet();
    let client = Eip155ExactClient::new(Arc::new(signer));
    let tag = Eip155Exact::price_tag(pay_to(), USDC::base().amount(1_000_000u64), None);
    let required = PaymentRequired::new(ResourceInfo::new("https://api.example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()]);
    let candidates = client.accept(&required);
    assert_eq!(candidates.len(), 1, "one eip3009 accept");
    let b64 = candidates
        .first()
        .expect("one eip3009 accept")
        .sign()
        .await
        .expect("local EIP-712 sign");
    let json = r402_protocol::payment::Base64Bytes(b64.into_bytes())
        .decode()
        .expect("b64");
    let payload: r402_evm::exact::payload::v2::PaymentPayload =
        serde_json::from_slice(&json).expect("payload");
    assert!(matches!(payload.payload, ExactPayload::Eip3009(_)));
    assert_eq!(payload.accepted.network.to_string(), "eip155:8453");
}

#[tokio::test]
async fn client_accepts_and_signs_permit2_locally() {
    let (signer, _) = wallet();
    let client = Eip155ExactClient::new(Arc::new(signer));
    let tag = Eip155Exact::price_tag(
        pay_to(),
        USDC::base().amount(1_000_000u64),
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
    let payload: r402_evm::exact::payload::v2::PaymentPayload =
        serde_json::from_slice(&json).expect("payload");
    assert!(matches!(payload.payload, ExactPayload::Permit2(_)));
}

#[test]
fn find_default_asset_base_usdc() {
    let network = "eip155:8453".parse().unwrap();
    let info = r402_evm::find_default_evm_asset_info(
        "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
        &network,
    )
    .expect("base usdc");
    assert_eq!(info.symbol, "USDC");
    assert_eq!(info.decimals, 6);
}
