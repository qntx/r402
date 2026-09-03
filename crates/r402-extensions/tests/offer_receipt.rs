//! Offer-receipt JWS / EIP-712 wire tests.

#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::panic,
    clippy::needless_collect,
    clippy::let_underscore_must_use,
    reason = "idiomatic test-code patterns"
)]

use std::sync::Arc;

use alloy_signer_local::PrivateKeySigner;
use r402_extensions::{
    Es256JwsSigner, OFFER_RECEIPT, OfferInput, OfferReceiptExtension, ReceiptInput,
    SignatureFormat, SignedOffer, canonicalize, create_jws,
    create_offer_eip712, create_offer_jws, create_receipt_eip712, create_receipt_jws,
    decode_signed_offers, extract_jws_header, extract_jws_payload, extract_offer_payload,
    extract_offers_from_payment_required, extract_receipt_from_settle_response,
    find_accepts_object_from_signed_offer, hash_offer_typed_data, verify_offer_signature_eip712,
    verify_offer_signature_jws, verify_receipt_signature_eip712,
};
use r402_protocol::extension::{AdvertiseContext, Extension, SettleContext};
use r402_protocol::payment::{
    Extensions, PaymentPayload, PaymentRequired, PaymentRequirements, ResourceInfo, SettleResponse,
};
use serde_json::json;

const SECRET: [u8; 32] = [0x11; 32];
const KID: &str = "did:web:example.com#key-1";
const RESOURCE: &str = "https://api.example.com/premium-data";
const ANVIL_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

fn sample_input() -> OfferInput {
    OfferInput {
        accept_index: 0,
        scheme: "exact".into(),
        network: "eip155:8453".into(),
        asset: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913".into(),
        pay_to: "0x1234567890123456789012345678901234567890".into(),
        amount: "10000".into(),
        offer_validity_seconds: Some(300),
    }
}

fn sample_accept() -> PaymentRequirements {
    PaymentRequirements::new(
        "exact".into(),
        "eip155:8453".parse().unwrap(),
        "10000".into(),
        "0x1234567890123456789012345678901234567890".into(),
        "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913".into(),
        300,
    )
}

#[test]
fn canonicalize_sorts_object_keys() {
    let v = json!({"z": 1, "a": 2});
    assert_eq!(canonicalize(&v).unwrap(), r#"{"a":2,"z":1}"#);
}

#[test]
fn jws_compact_has_three_parts_and_header() {
    let signer = Es256JwsSigner::from_bytes(KID, &SECRET).unwrap();
    let jws = create_jws(&json!({"test": "data", "number": 42}), &signer).unwrap();
    let parts: Vec<_> = jws.split('.').collect();
    assert_eq!(parts.len(), 3);
    let header = extract_jws_header(&jws).unwrap();
    assert_eq!(header.alg, "ES256");
    assert_eq!(header.kid.as_deref(), Some(KID));
}

#[test]
fn jws_payload_is_canonical() {
    let signer = Es256JwsSigner::from_bytes(KID, &SECRET).unwrap();
    let jws = create_jws(&json!({"z": 1, "a": 2}), &signer).unwrap();
    let payload: serde_json::Value = extract_jws_payload(&jws).unwrap();
    assert_eq!(payload, json!({"a": 2, "z": 1}));
}

#[test]
fn jws_offer_omits_payload_field() {
    let signer = Es256JwsSigner::from_bytes(KID, &SECRET).unwrap();
    let offer = create_offer_jws(RESOURCE, &sample_input(), &signer).unwrap();
    let encoded = serde_json::to_value(&offer).unwrap();
    assert_eq!(encoded["format"], "jws");
    assert!(encoded.get("payload").is_none());
    assert!(encoded["signature"].as_str().unwrap().contains('.'));
    let decoded = extract_offer_payload(&offer).unwrap();
    assert_eq!(decoded.resource_url, RESOURCE);
    assert_eq!(decoded.scheme, "exact");
}

#[test]
fn jws_offer_verifies_with_signer_key() {
    let signer = Es256JwsSigner::from_bytes(KID, &SECRET).unwrap();
    let offer = create_offer_jws(RESOURCE, &sample_input(), &signer).unwrap();
    let payload = verify_offer_signature_jws(&offer, Some(&signer.verifying_key())).unwrap();
    assert_eq!(payload.amount, "10000");
}

#[test]
fn eip712_offer_has_payload_and_hex_signature() {
    let wallet: PrivateKeySigner = ANVIL_KEY.parse().unwrap();
    let offer = create_offer_eip712(RESOURCE, &sample_input(), &wallet).unwrap();
    let SignedOffer::Eip712 {
        payload,
        signature,
        accept_index,
    } = &offer
    else {
        panic!("expected eip712");
    };
    assert_eq!(*accept_index, Some(0));
    assert!(signature.starts_with("0x"));
    assert_eq!(signature.len(), 132);
    assert_eq!(payload.resource_url, RESOURCE);
    let verified = verify_offer_signature_eip712(&offer).unwrap();
    assert_eq!(verified.signer, wallet.address());
    let _ = hash_offer_typed_data(payload);
}

#[test]
fn eip712_receipt_recovers_signer() {
    let wallet: PrivateKeySigner = ANVIL_KEY.parse().unwrap();
    let receipt = create_receipt_eip712(
        &ReceiptInput {
            resource_url: RESOURCE.into(),
            payer: "0x857b06519E91e3A54538791bDbb0E22373e36b66".into(),
            network: "eip155:8453".into(),
            transaction: None,
        },
        &wallet,
    )
    .unwrap();
    let verified = verify_receipt_signature_eip712(&receipt).unwrap();
    assert_eq!(verified.signer, wallet.address());
    assert_eq!(verified.payload.transaction.as_deref(), Some(""));
}

#[test]
fn jws_receipt_omits_transaction_when_unset() {
    let signer = Es256JwsSigner::from_bytes(KID, &SECRET).unwrap();
    let receipt = create_receipt_jws(
        &ReceiptInput {
            resource_url: RESOURCE.into(),
            payer: "0xpayer".into(),
            network: "eip155:8453".into(),
            transaction: None,
        },
        &signer,
    )
    .unwrap();
    let encoded = serde_json::to_value(&receipt).unwrap();
    assert!(encoded.get("payload").is_none());
    let payload =
        extract_jws_payload::<serde_json::Value>(encoded["signature"].as_str().unwrap()).unwrap();
    assert!(payload.get("transaction").is_none());
}

#[tokio::test]
async fn extension_enriches_402_with_signed_offers() {
    let signer = Es256JwsSigner::from_bytes(KID, &SECRET).unwrap();
    let issuer = r402_extensions::create_jws_offer_receipt_issuer(signer);
    let ext = OfferReceiptExtension::new(Arc::new(issuer));
    assert_eq!(ext.id(), OFFER_RECEIPT);
    let resource = ResourceInfo::new(RESOURCE);
    let accepts = vec![sample_accept()];
    let ctx = AdvertiseContext::for_payment_required(&resource, &accepts, None);
    let entry = ext.enrich_payment_required(&ctx).await.unwrap();
    let info = entry.as_info().unwrap();
    let offers = info["offers"].as_array().unwrap();
    assert_eq!(offers.len(), 1);
    assert_eq!(offers[0]["format"], "jws");
}

#[tokio::test]
async fn extension_adds_receipt_on_settle_success() {
    let signer = Es256JwsSigner::from_bytes(KID, &SECRET).unwrap();
    let issuer = r402_extensions::create_jws_offer_receipt_issuer(signer);
    let ext = OfferReceiptExtension::new(Arc::new(issuer)).with_include_tx_hash(true);
    let requirements = sample_accept();
    let payload =
        PaymentPayload::new(json!({}), json!({})).with_resource(ResourceInfo::new(RESOURCE));
    let response = SettleResponse::Success {
        payer: Some("0xpayer".into()),
        transaction: "0xabc".into(),
        network: "eip155:8453".into(),
        amount: None,
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    };
    let ctx = SettleContext::new(&payload, &requirements, &response);
    let entry = ext.on_settle(&ctx).await.unwrap();
    let info = entry.as_info().unwrap();
    assert_eq!(info["receipt"]["format"], "jws");
}

#[test]
fn extract_offers_and_match_accepts() {
    let signer = Es256JwsSigner::from_bytes(KID, &SECRET).unwrap();
    let offer = create_offer_jws(RESOURCE, &sample_input(), &signer).unwrap();
    let mut required =
        PaymentRequired::new(ResourceInfo::new(RESOURCE)).with_accepts(vec![sample_accept()]);
    required.extensions.insert(
        OFFER_RECEIPT,
        r402_protocol::payment::ExtensionEntry::info(json!({ "offers": [offer] })),
    );
    let offers = extract_offers_from_payment_required(&required);
    assert_eq!(offers.len(), 1);
    let decoded = decode_signed_offers(&offers).unwrap();
    let matched = find_accepts_object_from_signed_offer(
        &decoded[0].payload,
        decoded[0].accept_index,
        &required.accepts,
    );
    assert!(matched.is_some());
}

#[test]
fn extract_receipt_from_settle() {
    let signer = Es256JwsSigner::from_bytes(KID, &SECRET).unwrap();
    let receipt = create_receipt_jws(
        &ReceiptInput {
            resource_url: RESOURCE.into(),
            payer: "0xpayer".into(),
            network: "eip155:8453".into(),
            transaction: Some("0xabc".into()),
        },
        &signer,
    )
    .unwrap();
    let mut extensions = Extensions::new();
    extensions.insert(
        OFFER_RECEIPT,
        r402_protocol::payment::ExtensionEntry::info(json!({ "receipt": receipt })),
    );
    let response = SettleResponse::Success {
        payer: Some("0xpayer".into()),
        transaction: "0xabc".into(),
        network: "eip155:8453".into(),
        amount: None,
        extensions,
        extension_responses: Extensions::new(),
        extra: None,
    };
    assert!(extract_receipt_from_settle_response(&response).is_some());
}

#[test]
fn signature_format_roundtrip() {
    assert_eq!(
        serde_json::to_value(SignatureFormat::Jws).unwrap(),
        json!("jws")
    );
    assert_eq!(
        serde_json::to_value(SignatureFormat::Eip712).unwrap(),
        json!("eip712")
    );
}
