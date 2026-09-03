//! Builder-code (ERC-8021) declaration, client `s[]`, facilitator suffix.

#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::panic,
    clippy::format_collect,
    clippy::redundant_closure_for_method_calls,
    reason = "idiomatic test-code patterns"
)]

use std::future::Future;
use std::pin::Pin;

use r402_client::{
    ClientExtension, DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner, PaymentClient,
    SchemeClient,
};
use r402_extensions::{
    BUILDER_CODE, BuilderCodeClient, BuilderCodeData, BuilderCodeExtension,
    BuilderCodeFacilitatorExtension, ERC_8021_MARKER_HEX, MAX_CLIENT_SERVICE_CODES,
    MAX_SERVER_SERVICE_CODES, SCHEMA_2_ID, declare_builder_code_extension,
    encode_builder_code_suffix, parse_builder_code_suffix_from_calldata,
};
use r402_protocol::extension::{AdvertiseContext, Extension};
use r402_protocol::payment::{
    Base64Bytes, Extensions, PaymentRequired, PaymentRequirements, ResourceInfo,
};
use r402_protocol::{ChainId, ClientError, SchemeId};
use serde_json::{Value, json};

const APP: &str = "bc_my_app";
const SERVICE: &str = "bc_my_client";
const WALLET: &str = "bc_my_facilitator";

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn b64_json(value: &Value) -> String {
    Base64Bytes::encode(serde_json::to_vec(value).unwrap()).to_string()
}

fn json_from_b64(b64: &str) -> Value {
    serde_json::from_slice(&Base64Bytes::from(b64.as_bytes()).decode().unwrap()).unwrap()
}

fn required_bare() -> PaymentRequired {
    PaymentRequired::new(ResourceInfo::new("https://example.com")).with_accepts(vec![
        PaymentRequirements::new(
            "exact".into(),
            "eip155:1".parse().unwrap(),
            "1".into(),
            "0xb".into(),
            "0xa".into(),
            60,
        ),
    ])
}

fn builder_code_info(payload: &Value) -> &Value {
    &payload["extensions"][BUILDER_CODE]["info"]
}

struct JsonSigner(String);

impl PaymentCandidateSigner for JsonSigner {
    fn sign_payment<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + 'a>> {
        Box::pin(std::future::ready(Ok(self.0.clone())))
    }
}

struct JsonScheme;

impl SchemeId for JsonScheme {
    fn namespace(&self) -> &'static str {
        "eip155"
    }
    fn scheme(&self) -> &'static str {
        "exact"
    }
}

impl SchemeClient for JsonScheme {
    fn accept(&self, required: &PaymentRequired) -> Vec<PaymentCandidate> {
        required
            .accepts
            .iter()
            .map(|requirements| {
                let envelope = json!({
                    "x402Version": 2,
                    "accepted": {
                        "scheme": requirements.scheme,
                        "network": requirements.network.to_string(),
                        "amount": requirements.amount,
                        "payTo": requirements.pay_to,
                        "maxTimeoutSeconds": requirements.max_timeout_seconds,
                        "asset": requirements.asset,
                    },
                    "payload": { "signature": "signed" },
                });
                PaymentCandidate {
                    chain_id: requirements.network.clone(),
                    asset: requirements.asset.clone(),
                    amount: requirements.amount.clone(),
                    scheme: requirements.scheme.clone(),
                    pay_to: requirements.pay_to.clone(),
                    requirements: requirements.clone(),
                    signer: Box::new(JsonSigner(b64_json(&envelope))),
                }
            })
            .collect()
    }

    fn find_default_asset(&self, _: &str, _: &ChainId) -> Option<DefaultAssetInfo> {
        None
    }
}

#[test]
fn declare_rejects_invalid_app_code() {
    assert!(declare_builder_code_extension("INVALID", &[]).is_err());
}

#[test]
fn declare_omits_s_when_empty() {
    let entry = declare_builder_code_extension(APP, &[]).unwrap();
    assert_eq!(entry.as_info().unwrap()["a"], APP);
    assert!(entry.as_info().unwrap().get("s").is_none());
}

#[test]
fn declare_service_codes() {
    let entry = declare_builder_code_extension(APP, &["bc_server_sdk", "bc_other"]).unwrap();
    assert_eq!(
        entry.as_info().unwrap()["s"],
        json!(["bc_server_sdk", "bc_other"])
    );
}

#[test]
fn declare_rejects_too_many_server_codes() {
    let too_many: Vec<String> = (0..=MAX_SERVER_SERVICE_CODES)
        .map(|i| format!("bc_{i}"))
        .collect();
    let refs: Vec<&str> = too_many.iter().map(String::as_str).collect();
    assert!(declare_builder_code_extension(APP, &refs).is_err());
}

#[test]
fn advertise_from_extension() {
    let ext = BuilderCodeExtension::try_new(APP)
        .unwrap()
        .with_service_codes(["bc_sdk"])
        .unwrap();
    let entry = ext.advertise(&AdvertiseContext::new(None)).unwrap();
    assert_eq!(ext.id(), BUILDER_CODE);
    assert_eq!(entry.as_info().unwrap()["a"], APP);
    assert_eq!(entry.as_info().unwrap()["s"], json!(["bc_sdk"]));
}

#[test]
fn client_attaches_s_array() {
    let client = BuilderCodeClient::try_new([SERVICE, "bc_other"]).unwrap();
    let mut extensions = Extensions::new();
    client.enrich_extensions(&mut extensions);
    let info = extensions.get(BUILDER_CODE).unwrap().as_info().unwrap();
    assert_eq!(info["s"], json!([SERVICE, "bc_other"]));
}

#[test]
fn client_rejects_too_many() {
    let too_many: Vec<String> = (0..=MAX_CLIENT_SERVICE_CODES)
        .map(|i| format!("bc_{i}"))
        .collect();
    let refs: Vec<&str> = too_many.iter().map(String::as_str).collect();
    assert!(BuilderCodeClient::try_new(refs).is_err());
}

#[tokio::test]
async fn client_enrich_payment_payload_writes_s_not_a() {
    let client = BuilderCodeClient::try_new([SERVICE]).unwrap();
    let encoded = client
        .enrich_payment_payload(&b64_json(&json!({})), &required_bare())
        .await
        .unwrap();
    let decoded = json_from_b64(&encoded);
    let info = builder_code_info(&decoded);
    assert_eq!(info["s"], json!([SERVICE]));
    assert!(info.get("a").is_none());
}

#[tokio::test]
async fn client_enrich_merges_advertised_s() {
    let client = BuilderCodeClient::try_new([SERVICE]).unwrap();
    let mut advertised = Extensions::new();
    advertised.insert(
        BUILDER_CODE,
        declare_builder_code_extension(APP, &["bc_server"]).unwrap(),
    );
    let required = required_bare().with_extensions(advertised);
    let encoded = client
        .enrich_payment_payload(&b64_json(&json!({})), &required)
        .await
        .unwrap();
    let decoded = json_from_b64(&encoded);
    let info = builder_code_info(&decoded);
    assert_eq!(info["s"], json!([SERVICE, "bc_server"]));
    assert!(info.get("a").is_none());
}

#[tokio::test]
async fn client_enrich_does_not_insert_a_when_undeclared() {
    let client = BuilderCodeClient::try_new([SERVICE]).unwrap();
    let encoded = client
        .enrich_payment_payload(&b64_json(&json!({})), &required_bare())
        .await
        .unwrap();
    let decoded = json_from_b64(&encoded);
    let info = builder_code_info(&decoded);
    assert_eq!(info["s"], json!([SERVICE]));
    assert!(info.get("a").is_none());
}

#[tokio::test]
async fn client_enrich_leaves_existing_payload_a() {
    let client = BuilderCodeClient::try_new([SERVICE]).unwrap();
    let payload = json!({
        "extensions": {
            "builder-code": { "info": { "a": "bc_forged" } }
        }
    });
    let encoded = client
        .enrich_payment_payload(&b64_json(&payload), &required_bare())
        .await
        .unwrap();
    let decoded = json_from_b64(&encoded);
    let info = builder_code_info(&decoded);
    assert_eq!(info["a"], "bc_forged");
    assert_eq!(info["s"], json!([SERVICE]));
}

#[tokio::test]
async fn client_enrich_json_error() {
    let client = BuilderCodeClient::try_new([SERVICE]).unwrap();
    let required = required_bare();
    let parse_err = client
        .enrich_payment_payload("!!!", &required)
        .await
        .unwrap_err();
    assert!(
        matches!(parse_err, ClientError::Parse(_)),
        "invalid base64 must be Parse, got {parse_err:?}"
    );
    let json_err = client
        .enrich_payment_payload(&Base64Bytes::encode(b"{").to_string(), &required)
        .await
        .unwrap_err();
    assert!(
        matches!(json_err, ClientError::Json(_)),
        "invalid JSON must be Json, got {json_err:?}"
    );
}

#[tokio::test]
async fn client_enrich_non_object_payload_is_parse() {
    let client = BuilderCodeClient::try_new([SERVICE]).unwrap();
    let required = required_bare();
    for payload in [json!([]), json!("nope")] {
        let err = client
            .enrich_payment_payload(&b64_json(&payload), &required)
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                ClientError::Parse(ref msg) if msg == "payment payload is not a JSON object"
            ),
            "non-object payload must be Parse, got {err:?}"
        );
    }
}

#[tokio::test]
async fn client_enrich_missing_info_creates_s_only() {
    let client = BuilderCodeClient::try_new([SERVICE]).unwrap();
    let payload = json!({
        "extensions": {
            "builder-code": { "schema": { "type": "object" } }
        }
    });
    let encoded = client
        .enrich_payment_payload(&b64_json(&payload), &required_bare())
        .await
        .unwrap();
    let decoded = json_from_b64(&encoded);
    let info = builder_code_info(&decoded);
    assert_eq!(info["s"], json!([SERVICE]));
    assert!(info.get("a").is_none());
    assert_eq!(
        decoded["extensions"][BUILDER_CODE]["schema"],
        json!({ "type": "object" })
    );
}

#[test]
fn client_registers_via_with_extension() {
    let client =
        PaymentClient::new().with_extension(BuilderCodeClient::try_new([SERVICE]).unwrap());
    assert_eq!(client.extension_count(), 1);
}

#[tokio::test]
async fn create_payment_enriches_s() {
    let created = PaymentClient::new()
        .disable_spend_controls()
        .register(JsonScheme)
        .with_extension(BuilderCodeClient::try_new([SERVICE]).unwrap())
        .create_payment(&required_bare())
        .await
        .unwrap();
    let decoded = json_from_b64(&created.signed_payload);
    let info = builder_code_info(&decoded);
    assert_eq!(info["s"], json!([SERVICE]));
    assert!(info.get("a").is_none());
}

#[test]
fn suffix_roundtrip_all_fields() {
    let data = BuilderCodeData {
        a: Some(APP.into()),
        w: Some(WALLET.into()),
        s: vec![SERVICE.into()],
    };
    let suffix = encode_builder_code_suffix(&data);
    let mut calldata = vec![0xde, 0xad, 0xbe, 0xef];
    calldata.extend_from_slice(&suffix);
    assert_eq!(
        parse_builder_code_suffix_from_calldata(&calldata).unwrap(),
        data
    );
}

#[test]
fn suffix_matches_spec_app_only_fixture() {
    let suffix = encode_builder_code_suffix(&BuilderCodeData {
        a: Some("bc_myapp".into()),
        w: None,
        s: vec![],
    });
    let hex = hex_lower(&suffix);
    assert!(hex.ends_with(&format!("000c02{ERC_8021_MARKER_HEX}")));
    assert_eq!(
        hex,
        format!("a161616862635f6d79617070000c02{ERC_8021_MARKER_HEX}")
    );
    assert_eq!(suffix[suffix.len() - 17], SCHEMA_2_ID);
}

#[test]
fn suffix_matches_spec_app_and_facilitator_fixture() {
    let suffix = encode_builder_code_suffix(&BuilderCodeData {
        a: Some("bc_myapp".into()),
        w: Some("bc_myfacilitator".into()),
        s: vec![],
    });
    let hex = hex_lower(&suffix);
    assert_eq!(
        hex,
        format!(
            "a261616862635f6d7961707061777062635f6d79666163696c697461746f72001f02{ERC_8021_MARKER_HEX}"
        )
    );
}

#[test]
fn parse_returns_none_without_marker() {
    assert!(parse_builder_code_suffix_from_calldata(&[0xde, 0xad]).is_none());
}

#[test]
fn facilitator_encodes_w_only_when_configured() {
    let ext = BuilderCodeFacilitatorExtension::new()
        .with_builder_code(WALLET)
        .unwrap();
    let suffix = ext.build_data_suffix(&Extensions::new(), 2).unwrap();
    let parsed = parse_builder_code_suffix_from_calldata(&suffix).unwrap();
    assert_eq!(parsed.w.as_deref(), Some(WALLET));
    assert!(parsed.a.is_none());
}

#[test]
fn facilitator_omits_suffix_when_empty() {
    let ext = BuilderCodeFacilitatorExtension::new();
    assert!(ext.build_data_suffix(&Extensions::new(), 2).is_none());
}

#[test]
fn facilitator_reads_payload_info() {
    let ext = BuilderCodeFacilitatorExtension::new()
        .with_builder_code(WALLET)
        .unwrap();
    let mut extensions = Extensions::new();
    extensions.insert(
        BUILDER_CODE,
        r402_protocol::payment::ExtensionEntry::info(json!({ "a": APP, "s": SERVICE })),
    );
    let suffix = ext.build_data_suffix(&extensions, 2).unwrap();
    let parsed = parse_builder_code_suffix_from_calldata(&suffix).unwrap();
    assert_eq!(parsed.a.as_deref(), Some(APP));
    assert_eq!(parsed.w.as_deref(), Some(WALLET));
    assert_eq!(
        parsed.s.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        vec![SERVICE]
    );
}

#[test]
fn facilitator_drops_a_on_v1_keeps_s() {
    let ext = BuilderCodeFacilitatorExtension::new()
        .with_builder_code(WALLET)
        .unwrap()
        .with_service_code("bc_fac")
        .unwrap();
    let mut extensions = Extensions::new();
    extensions.insert(
        BUILDER_CODE,
        r402_protocol::payment::ExtensionEntry::info(json!({ "a": APP, "s": SERVICE })),
    );
    let suffix = ext.build_data_suffix(&extensions, 1).unwrap();
    let parsed = parse_builder_code_suffix_from_calldata(&suffix).unwrap();
    assert!(parsed.a.is_none());
    assert_eq!(
        parsed.s.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        vec![SERVICE, "bc_fac"]
    );
}

#[test]
fn facilitator_truncates_echoed_s_to_ten() {
    let ext = BuilderCodeFacilitatorExtension::new()
        .with_builder_code(WALLET)
        .unwrap();
    let codes: Vec<String> = (1..=11).map(|i| format!("bc_{i}")).collect();
    let mut extensions = Extensions::new();
    extensions.insert(
        BUILDER_CODE,
        r402_protocol::payment::ExtensionEntry::info(json!({ "s": codes })),
    );
    let suffix = ext.build_data_suffix(&extensions, 2).unwrap();
    let parsed = parse_builder_code_suffix_from_calldata(&suffix).unwrap();
    assert_eq!(parsed.s.len(), 10);
    assert_eq!(parsed.s[0].as_str(), "bc_1");
    assert_eq!(parsed.s[9].as_str(), "bc_10");
}

#[test]
fn facilitator_dedupes_own_service_code() {
    let ext = BuilderCodeFacilitatorExtension::new()
        .with_builder_code(WALLET)
        .unwrap()
        .with_service_code(SERVICE)
        .unwrap();
    let mut extensions = Extensions::new();
    extensions.insert(
        BUILDER_CODE,
        r402_protocol::payment::ExtensionEntry::info(json!({ "s": [SERVICE] })),
    );
    let suffix = ext.build_data_suffix(&extensions, 2).unwrap();
    let parsed = parse_builder_code_suffix_from_calldata(&suffix).unwrap();
    assert_eq!(
        parsed.s.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        vec![SERVICE]
    );
}

#[test]
fn facilitator_drops_invalid_s_entries() {
    let ext = BuilderCodeFacilitatorExtension::new()
        .with_builder_code(WALLET)
        .unwrap();
    let mut extensions = Extensions::new();
    extensions.insert(
        BUILDER_CODE,
        r402_protocol::payment::ExtensionEntry::info(
            json!({ "s": ["INVALID", SERVICE, "bc_other"] }),
        ),
    );
    let suffix = ext.build_data_suffix(&extensions, 2).unwrap();
    let parsed = parse_builder_code_suffix_from_calldata(&suffix).unwrap();
    assert_eq!(
        parsed.s.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        vec![SERVICE, "bc_other"]
    );
}
