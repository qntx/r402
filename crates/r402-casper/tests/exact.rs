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
    clippy::panic,
    reason = "idiomatic test-code patterns"
)]

//! Offline exact-scheme tests. No live Casper RPC. No HTTP E2E.

use r402_casper::chain::CasperChainReference;
use r402_casper::{CASPER_NETWORKS, CasperExact, WCSPR};
use r402_protocol::scheme::SchemeId;

#[test]
fn crate_name_matches_directory() {
    assert_eq!(env!("CARGO_PKG_NAME"), "r402-casper");
}

#[test]
fn casper_exact_scheme_id() {
    assert_eq!(CasperExact.namespace(), "casper");
    assert_eq!(CasperExact.scheme(), "exact");
    assert_eq!(CasperExact.caip_family(), "casper:*");
}

#[test]
fn networks_table_has_mainnet_and_testnet() {
    assert_eq!(CASPER_NETWORKS.len(), 2);
    let names: Vec<_> = CASPER_NETWORKS.iter().map(|n| n.name).collect();
    assert_eq!(names, ["casper", "casper-test"]);
}

#[test]
fn chain_reference_converts_to_caip2() {
    let chain: r402_protocol::ChainId = CasperChainReference::CASPER_TEST.into();
    assert_eq!(chain.to_string(), "casper:casper-test");
}

#[test]
fn wcspr_testnet_package_hash_is_untagged() {
    let deployment = WCSPR::casper_test();
    assert_eq!(deployment.decimals, 9);
    assert_eq!(
        deployment.address.to_string(),
        "3d80df21ba4ee4d66a2a1f60c32570dd5685e4b279f6538162a5fd1314847c1e"
    );
    assert!(WCSPR::on(CasperChainReference::CASPER).is_none());
}

#[cfg(feature = "client")]
#[test]
fn find_default_casper_asset_covers_wcspr() {
    use r402_casper::find_default_casper_asset;
    use r402_protocol::ChainId;

    let testnet: ChainId = "casper:casper-test".parse().expect("testnet");
    let wcspr = find_default_casper_asset(
        "3d80df21ba4ee4d66a2a1f60c32570dd5685e4b279f6538162a5fd1314847c1e",
        &testnet,
    )
    .expect("wCSPR");
    assert_eq!(wcspr.symbol, "wCSPR");
    assert_eq!(wcspr.decimals, 9);
    assert!(find_default_casper_asset("00ab", &testnet).is_none());
}

#[cfg(feature = "server")]
#[test]
fn price_tag_carries_eip712_domain() {
    use r402_casper::chain::Address;

    let pay_to: Address = "00fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321"
        .parse()
        .expect("payTo");
    let tag = CasperExact::price_tag(pay_to, WCSPR::casper_test().amount(1_500_000_000));
    assert_eq!(tag.requirements.scheme, "exact");
    assert_eq!(tag.requirements.network.to_string(), "casper:casper-test");
    assert_eq!(tag.requirements.amount, "1500000000");
    let extra = tag.requirements.extra.as_ref().expect("eip712 extra");
    assert_eq!(extra["name"], "Wrapped CSPR");
    assert_eq!(extra["version"], "1");
}

#[cfg(feature = "facilitator")]
fn try_new_question_mark() -> Result<(), r402_protocol::FacilitatorError> {
    use r402_casper::ReqwestTransport;
    use r402_casper::exact::CasperExactFacilitator;

    let _fac = CasperExactFacilitator::try_new(ReqwestTransport::new())?;
    Ok(())
}

#[cfg(feature = "facilitator")]
#[test]
fn try_new_question_mark_compiles() {
    try_new_question_mark().expect("try_new is currently infallible");
}

#[cfg(feature = "facilitator")]
#[tokio::test]
async fn try_new_supported_hits_hosted_endpoint_shape() {
    use r402_casper::exact::CasperExactFacilitator;
    use r402_casper::{CasperFacilitatorConfig, FACILITATOR_URL_ENV};

    let fac = CasperExactFacilitator::hosted_http();
    assert_eq!(
        fac.config().verify_url().as_str(),
        "https://x402-facilitator.cspr.cloud/verify"
    );
    assert_eq!(FACILITATOR_URL_ENV, "R402_CASPER_FACILITATOR_URL");
    let _ = CasperFacilitatorConfig::from_env().expect("default env config");
}

#[cfg(feature = "client")]
#[test]
fn payload_shape_binds_public_key_to_from() {
    use r402_casper::exact::{ExactCasperPayload, validate_payload_shape};

    let json = serde_json::json!({
        "signature": format!("02{}", "aa".repeat(64)),
        "publicKey": "020376e4f8766e4f33bcc6e20b331b5163f363dc0106063b052ad38afe08637bd867",
        "authorization": {
            "from": "0076d080b4e769f0b29c77fc6472d6e425710840c2f46a4506e5544d2ce34f43a3",
            "to": "00fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321",
            "value": "1500000000",
            "validAfter": "1700000000",
            "validBefore": "1700000600",
            "nonce": "cc".repeat(32),
        }
    });
    let payload: ExactCasperPayload = serde_json::from_value(json).unwrap();
    validate_payload_shape(&payload).expect("fixture binds publicKey to from");
}
