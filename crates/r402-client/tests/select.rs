#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::indexing_slicing,
    clippy::missing_const_for_fn,
    clippy::shadow_unrelated,
    clippy::unwrap_used,
    reason = "idiomatic test-code patterns"
)]

//! Selection, `paymentFlow` preference, selectors, policies, and hooks.

use std::future::Future;
use std::pin::Pin;

use http::{HeaderMap, HeaderValue};
use r402_client::{
    ClientExtension, ClientHooks, DefaultAssetInfo, FirstMatch, HookDecision, MaxAmount,
    MaxAmountPolicy, NetworkPolicy, PaymentCandidate, PaymentCandidateSigner, PaymentClient,
    PaymentCreationContext, PaymentPolicy, PaymentResponseContext, PaymentResponseResult,
    PaymentSelector, PreferChain, SchemeClient, SchemePolicy,
};
use r402_protocol::payment::ExtensionEntry;
use r402_protocol::{
    ChainId, ClientError, PaymentRequired, PaymentRequirements, ResourceInfo, SchemeId,
};

const USDC: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
const NETWORK: &str = "eip155:8453";

struct StubSigner(String);
impl PaymentCandidateSigner for StubSigner {
    fn sign_payment<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + 'a>> {
        let payload = self.0.clone();
        Box::pin(async move { Ok(payload) })
    }
}

struct StubScheme {
    default_asset: Option<DefaultAssetInfo>,
}

impl StubScheme {
    fn bare() -> Self {
        Self {
            default_asset: None,
        }
    }
}

impl SchemeId for StubScheme {
    fn namespace(&self) -> &'static str {
        "eip155"
    }
    fn scheme(&self) -> &'static str {
        "exact"
    }
}
impl SchemeClient for StubScheme {
    fn accept(&self, required: &PaymentRequired) -> Vec<PaymentCandidate> {
        required
            .accepts
            .iter()
            .map(|r| PaymentCandidate {
                chain_id: r.network.clone(),
                asset: r.asset.clone(),
                amount: r.amount.clone(),
                scheme: r.scheme.clone(),
                pay_to: r.pay_to.clone(),
                requirements: r.clone(),
                signer: Box::new(StubSigner("c2lnbmVk".into())),
            })
            .collect()
    }

    fn find_default_asset(&self, asset: &str, _network: &ChainId) -> Option<DefaultAssetInfo> {
        self.default_asset
            .as_ref()
            .filter(|info| info.asset.eq_ignore_ascii_case(asset))
            .cloned()
    }
}

struct AbortHook;
impl ClientHooks for AbortHook {
    fn before_payment_creation<'a>(
        &'a self,
        _: &PaymentCreationContext,
    ) -> impl Future<Output = HookDecision> + Send + 'a {
        std::future::ready(HookDecision::Abort {
            reason: "nope".into(),
            message: String::new(),
        })
    }
}

struct RecoverHook;
impl ClientHooks for RecoverHook {
    fn on_payment_response<'a>(
        &'a self,
        _: &PaymentResponseContext,
    ) -> impl Future<Output = PaymentResponseResult> + Send + 'a {
        std::future::ready(PaymentResponseResult::recovered())
    }
}

struct PrefixExt;
impl ClientExtension for PrefixExt {
    fn key(&self) -> &'static str {
        "builder-code"
    }

    fn enrich_payment_payload<'a>(
        &'a self,
        payload_b64: &'a str,
        _: &'a PaymentRequired,
    ) -> impl Future<Output = Result<String, ClientError>> + Send + 'a {
        std::future::ready(Ok(format!("x{payload_b64}")))
    }
}

struct HeaderExt;
impl ClientExtension for HeaderExt {
    fn key(&self) -> &'static str {
        "sign-in-with-x"
    }

    fn on_payment_required<'a>(
        &'a self,
        _: &'a PaymentRequired,
    ) -> impl Future<Output = HeaderMap> + Send + 'a {
        let mut headers = HeaderMap::new();
        let _ = headers.insert("SIGN-IN-WITH-X", HeaderValue::from_static("cHJvb2Y="));
        std::future::ready(headers)
    }
}

fn sample_required() -> PaymentRequired {
    required_with("eip155:1", "0xa", "1", None)
}

fn required_with(
    network: &str,
    asset: &str,
    amount: &str,
    extra: Option<serde_json::Value>,
) -> PaymentRequired {
    required_accepts(vec![accept(network, asset, amount, extra)])
}

fn required_accepts(accepts: Vec<PaymentRequirements>) -> PaymentRequired {
    PaymentRequired::new(ResourceInfo::new("https://example.com")).with_accepts(accepts)
}

fn accept(
    network: &str,
    asset: &str,
    amount: &str,
    extra: Option<serde_json::Value>,
) -> PaymentRequirements {
    let mut req = PaymentRequirements::new(
        "exact".into(),
        network.parse::<ChainId>().unwrap(),
        amount.into(),
        "0xb".into(),
        asset.into(),
        60,
    );
    if let Some(extra) = extra {
        req = req.with_extra(extra);
    }
    req
}

#[tokio::test]
async fn create_payment_signs() {
    let client = PaymentClient::new()
        .disable_spend_controls()
        .register(StubScheme::bare());
    let created = client.create_payment(&sample_required()).await.unwrap();
    assert_eq!(
        created.signed_payload, "c2lnbmVk",
        "stub signer payload must round-trip"
    );
}

#[tokio::test]
async fn before_hook_aborts() {
    let client = PaymentClient::new()
        .disable_spend_controls()
        .register(StubScheme::bare())
        .with_hook(AbortHook);
    let err = client.create_payment(&sample_required()).await.unwrap_err();
    assert!(
        matches!(err, ClientError::Parse(ref s) if s.contains("nope")),
        "abort hook must surface as parse error, got {err:?}"
    );
}

#[tokio::test]
async fn payment_response_hook_recovers() {
    let client = PaymentClient::new().with_hook(RecoverHook);
    let ctx = PaymentResponseContext::new(sample_required(), "x")
        .with_corrective_payment_required(sample_required());
    let result = client.handle_payment_response(&ctx).await;
    assert!(result.recovered, "recover hook must set recovered");
}

#[tokio::test]
async fn no_schemes_yields_no_match() {
    let client = PaymentClient::new();
    let err = client.create_payment(&sample_required()).await.unwrap_err();
    assert!(
        matches!(err, ClientError::NoMatchingPaymentOption),
        "empty registry must not match, got {err:?}"
    );
}

#[tokio::test]
async fn drops_unrecognized_payment_flow_and_prefers_authorization() {
    let client = PaymentClient::new()
        .disable_spend_controls()
        .register(StubScheme::bare());
    let required = required_accepts(vec![
        accept(
            NETWORK,
            USDC,
            "200",
            Some(serde_json::json!({"paymentFlow": "future-flow"})),
        ),
        accept(NETWORK, USDC, "100", None),
    ]);
    let recognized = client.candidates(&required);
    let picked = client.select_candidate(&recognized).unwrap();
    assert_eq!(picked.amount, "100", "unrecognized flow must be dropped");
}

#[tokio::test]
async fn all_unrecognized_payment_flow_errors() {
    let client = PaymentClient::new()
        .disable_spend_controls()
        .register(StubScheme::bare());
    let err = client
        .create_payment(&required_accepts(vec![accept(
            NETWORK,
            USDC,
            "1",
            Some(serde_json::json!({"paymentFlow": "future-flow"})),
        )]))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ClientError::UnrecognizedPaymentFlow),
        "expected UnrecognizedPaymentFlow, got {err:?}"
    );
}

#[tokio::test]
async fn prefers_omitted_authorization_over_upfront() {
    let client = PaymentClient::new()
        .disable_spend_controls()
        .register(StubScheme::bare());
    let mixed = required_accepts(vec![
        accept(
            NETWORK,
            USDC,
            "100",
            Some(serde_json::json!({"paymentFlow": "upfront"})),
        ),
        accept(NETWORK, USDC, "200", None),
    ]);
    let mixed_candidates = client.candidates(&mixed);
    let mixed_selected = client.select_candidate(&mixed_candidates).unwrap();
    assert_eq!(
        mixed_selected.amount, "200",
        "omitted paymentFlow is authorization"
    );
}

#[tokio::test]
async fn prefers_explicit_authorization_over_escrow() {
    let client = PaymentClient::new()
        .disable_spend_controls()
        .register(StubScheme::bare());
    let escrow_and_auth = required_accepts(vec![
        accept(
            NETWORK,
            USDC,
            "100",
            Some(serde_json::json!({"paymentFlow": "escrow"})),
        ),
        accept(
            NETWORK,
            USDC,
            "200",
            Some(serde_json::json!({"paymentFlow": "authorization"})),
        ),
    ]);
    let auth_candidates = client.candidates(&escrow_and_auth);
    let auth_selected = client.select_candidate(&auth_candidates).unwrap();
    assert_eq!(
        auth_selected.amount, "200",
        "explicit authorization must win over escrow"
    );
}

#[tokio::test]
async fn selects_upfront_when_it_is_the_only_accept() {
    let client = PaymentClient::new()
        .disable_spend_controls()
        .register(StubScheme::bare());
    let upfront_only = required_accepts(vec![accept(
        NETWORK,
        USDC,
        "100",
        Some(serde_json::json!({"paymentFlow": "upfront"})),
    )]);
    let upfront_candidates = client.candidates(&upfront_only);
    let upfront_selected = client.select_candidate(&upfront_candidates).unwrap();
    assert_eq!(
        upfront_selected.amount, "100",
        "sole recognized non-auth flow must still be selectable"
    );
}

struct UpfrontOnly;
impl PaymentPolicy for UpfrontOnly {
    fn apply<'a>(&self, candidates: Vec<&'a PaymentCandidate>) -> Vec<&'a PaymentCandidate> {
        candidates
            .into_iter()
            .filter(|candidate| {
                candidate
                    .requirements
                    .extra
                    .as_ref()
                    .and_then(|extra| extra.get("paymentFlow"))
                    .and_then(serde_json::Value::as_str)
                    == Some("upfront")
            })
            .collect()
    }
}

#[tokio::test]
async fn policy_can_override_authorization_preference() {
    let client = PaymentClient::new()
        .disable_spend_controls()
        .register(StubScheme::bare())
        .with_policy(UpfrontOnly);
    let required = required_accepts(vec![
        accept(NETWORK, USDC, "200", None),
        accept(
            NETWORK,
            USDC,
            "100",
            Some(serde_json::json!({"paymentFlow": "upfront"})),
        ),
    ]);
    let candidates = client.candidates(&required);
    let selected = client.select_candidate(&candidates).unwrap();
    assert_eq!(
        selected.amount, "100",
        "policy filter runs before authorization preference"
    );
}

#[tokio::test]
async fn prefer_chain_picks_matching_network() {
    let client = PaymentClient::new()
        .disable_spend_controls()
        .register(StubScheme::bare())
        .with_selector(PreferChain::new(vec!["eip155:8453".parse().unwrap()]));
    let required = required_accepts(vec![
        accept("eip155:1", USDC, "100", None),
        accept(NETWORK, USDC, "200", None),
    ]);
    let candidates = client.candidates(&required);
    let selected = client.select_candidate(&candidates).unwrap();
    assert_eq!(
        selected.amount, "200",
        "PreferChain must pick the preferred network even when second"
    );
}

#[tokio::test]
async fn max_amount_selector_skips_over_budget() {
    let client = PaymentClient::new()
        .disable_spend_controls()
        .register(StubScheme::bare())
        .with_selector(MaxAmount(150));
    let required = required_accepts(vec![
        accept(NETWORK, USDC, "200", None),
        accept(NETWORK, USDC, "100", None),
    ]);
    let candidates = client.candidates(&required);
    let selected = client.select_candidate(&candidates).unwrap();
    assert_eq!(
        selected.amount, "100",
        "MaxAmount must skip the first over-budget accept"
    );
}

#[tokio::test]
async fn network_and_scheme_policies_filter() {
    let client = PaymentClient::new()
        .disable_spend_controls()
        .register(StubScheme::bare())
        .with_policy(NetworkPolicy::new(vec!["eip155:8453".parse().unwrap()]))
        .with_policy(SchemePolicy::new(["exact"]))
        .with_policy(MaxAmountPolicy(150));
    let required = required_accepts(vec![
        accept("eip155:1", USDC, "50", None),
        accept(NETWORK, USDC, "200", None),
        accept(NETWORK, USDC, "100", None),
    ]);
    let candidates = client.candidates(&required);
    let selected = client.select_candidate(&candidates).unwrap();
    assert_eq!(
        selected.amount, "100",
        "policies must drop wrong network and over-budget amounts"
    );
}

#[test]
fn first_match_returns_first() {
    let client = PaymentClient::new()
        .disable_spend_controls()
        .register(StubScheme::bare());
    let required = required_accepts(vec![
        accept(NETWORK, USDC, "1", None),
        accept(NETWORK, USDC, "2", None),
    ]);
    let candidates = client.candidates(&required);
    let refs: Vec<&PaymentCandidate> = candidates.iter().collect();
    let picked = FirstMatch.select(&refs).unwrap();
    assert_eq!(picked.amount, "1", "FirstMatch must keep list order");
}

#[tokio::test]
async fn enrich_runs_after_sign() {
    let client = PaymentClient::new()
        .disable_spend_controls()
        .register(StubScheme::bare())
        .with_extension(PrefixExt);
    let created = client.create_payment(&sample_required()).await.unwrap();
    assert_eq!(created.signed_payload, "xc2lnbmVk");
}

#[tokio::test]
async fn extension_headers_require_declaration() {
    let client = PaymentClient::new().with_extension(HeaderExt);
    let empty = client.extension_headers(&sample_required()).await;
    assert!(
        empty.is_empty(),
        "undeclared extension must not emit headers"
    );

    let mut declared = sample_required();
    declared.extensions.insert(
        "sign-in-with-x",
        ExtensionEntry::info(serde_json::json!({})),
    );
    let headers = client.extension_headers(&declared).await;
    assert_eq!(
        headers.get("SIGN-IN-WITH-X").map(HeaderValue::as_bytes),
        Some(b"cHJvb2Y=".as_slice())
    );
}
