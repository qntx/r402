//! Buyer 402 retry. 412 and 502 must not retry.

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

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use compact_str::CompactString;
use http::{HeaderMap, HeaderValue};
use r402_client::{
    ClientExtension, ClientHooks, DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner,
    PaymentResponseContext, PaymentResponseResult, SchemeClient,
};
use r402_extensions::siwx::{
    SIWX_KEY, SiwxChain, SiwxClientExtension, SiwxError, SiwxExtension, SiwxOrigin, SiwxSigner,
};
use r402_http::{PAYMENT_REQUIRED, PAYMENT_SIGNATURE, SIGN_IN_WITH_X, WithPayments, X402Client};
use r402_protocol::payment::{
    Base64Bytes, ExtensionEntry, PaymentRequired, PaymentRequirements, ResourceInfo,
};
use r402_protocol::{ChainId, ClientError, SchemeId};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SIGNED: &str = "c2lnbmVk";
const FIXTURE_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

struct StubSigner;
impl PaymentCandidateSigner for StubSigner {
    fn sign_payment<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + 'a>> {
        Box::pin(async { Ok(SIGNED.to_owned()) })
    }
}

struct StubScheme;
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
                signer: Box::new(StubSigner),
            })
            .collect()
    }

    fn find_default_asset(&self, _asset: &str, _network: &ChainId) -> Option<DefaultAssetInfo> {
        None
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

struct SignInHeaderExt;
impl ClientExtension for SignInHeaderExt {
    fn key(&self) -> &'static str {
        "sign-in-with-x"
    }

    fn on_payment_required<'a>(
        &'a self,
        _: &'a PaymentRequired,
        _: &'a str,
    ) -> impl Future<Output = HeaderMap> + Send + 'a {
        let mut headers = HeaderMap::new();
        let _ = headers.insert(SIGN_IN_WITH_X, HeaderValue::from_static("cHJvb2Y="));
        std::future::ready(headers)
    }
}

struct CaptureUrlExt {
    urls: Arc<Mutex<Vec<String>>>,
}

impl ClientExtension for CaptureUrlExt {
    fn key(&self) -> &'static str {
        "sign-in-with-x"
    }

    fn on_payment_required<'a>(
        &'a self,
        _: &'a PaymentRequired,
        request_url: &'a str,
    ) -> impl Future<Output = HeaderMap> + Send + 'a {
        self.urls.lock().unwrap().push(request_url.to_owned());
        let mut headers = HeaderMap::new();
        let _ = headers.insert(SIGN_IN_WITH_X, HeaderValue::from_static("cHJvb2Y="));
        std::future::ready(headers)
    }
}

struct FixtureEvm(PrivateKeySigner);

impl SiwxSigner for FixtureEvm {
    fn signature_type(&self) -> &'static str {
        "eip191"
    }

    fn address(&self) -> CompactString {
        format!("{}", self.0.address()).into()
    }

    fn sign_message<'a>(
        &'a self,
        message: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<CompactString, SiwxError>> + Send + 'a>> {
        Box::pin(async move {
            let sig = self
                .0
                .sign_message(message.as_bytes())
                .await
                .map_err(|_| SiwxError::Signature)?;
            Ok(format!("{sig}").into())
        })
    }
}

struct RecordingSiwx {
    inner: SiwxClientExtension,
    urls: Arc<Mutex<Vec<String>>>,
}

impl RecordingSiwx {
    fn new(urls: Arc<Mutex<Vec<String>>>) -> Self {
        let signer: PrivateKeySigner = FIXTURE_KEY.parse().unwrap();
        Self {
            inner: SiwxClientExtension::new().with_signer(FixtureEvm(signer)),
            urls,
        }
    }
}

impl ClientExtension for RecordingSiwx {
    fn key(&self) -> &'static str {
        SIWX_KEY
    }

    fn on_payment_required<'a>(
        &'a self,
        payment_required: &'a PaymentRequired,
        request_url: &'a str,
    ) -> impl Future<Output = HeaderMap> + Send + 'a {
        self.urls.lock().unwrap().push(request_url.to_owned());
        self.inner
            .on_payment_required(payment_required, request_url)
    }
}

fn sample_required() -> PaymentRequired {
    PaymentRequired::new(ResourceInfo::new("https://example.com/paid")).with_accepts(vec![
        PaymentRequirements::new(
            "exact".into(),
            "eip155:8453".parse().unwrap(),
            "1000000".into(),
            "0xb".into(),
            "0xa".into(),
            60,
        ),
    ])
}

fn sample_required_with_siwx() -> PaymentRequired {
    let mut required = sample_required();
    required.extensions.insert(
        "sign-in-with-x",
        ExtensionEntry::raw(serde_json::json!({
            "info": { "domain": "example.com" },
            "supportedChains": []
        })),
    );
    required
}

fn b64_json(value: &PaymentRequired) -> String {
    let json = serde_json::to_vec(value).unwrap();
    String::from_utf8(Base64Bytes::encode(json).0).unwrap()
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

fn paid_buyer() -> X402Client {
    X402Client::new()
        .disable_spend_controls()
        .register(StubScheme)
}

#[tokio::test]
async fn retries_402_with_payment_signature() {
    let server = MockServer::start().await;
    let challenge = b64_json(&sample_required());

    Mock::given(method("GET"))
        .and(path("/paid"))
        .and(header(PAYMENT_SIGNATURE, SIGNED))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/paid"))
        .respond_with(
            ResponseTemplate::new(402).insert_header(PAYMENT_REQUIRED, challenge.as_str()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = http_client().with_payments(paid_buyer());
    let response = client
        .get(format!("{}/paid", server.uri()))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        200,
        "402 must retry with Payment-Signature"
    );
    assert_eq!(response.text().await.unwrap(), "ok");
}

#[tokio::test]
async fn retries_402_from_body_without_header() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/paid"))
        .and(header(PAYMENT_SIGNATURE, SIGNED))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/paid"))
        .respond_with(ResponseTemplate::new(402).set_body_json(sample_required()))
        .expect(1)
        .mount(&server)
        .await;

    let client = http_client().with_payments(paid_buyer());
    let response = client
        .get(format!("{}/paid", server.uri()))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        200,
        "402 body without Payment-Required header must still retry"
    );
}

#[tokio::test]
async fn does_not_retry_412() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/paid"))
        .respond_with(ResponseTemplate::new(412))
        .expect(1)
        .mount(&server)
        .await;

    let client = http_client().with_payments(paid_buyer());
    let response = client
        .get(format!("{}/paid", server.uri()))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        412,
        "412 Permit2 must be returned without a paid retry"
    );
}

#[tokio::test]
async fn does_not_retry_502() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/paid"))
        .respond_with(ResponseTemplate::new(502))
        .expect(1)
        .mount(&server)
        .await;

    let client = http_client().with_payments(paid_buyer());
    let response = client
        .get(format!("{}/paid", server.uri()))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        502,
        "502 transport must be returned without a paid retry"
    );
}

#[tokio::test]
async fn recovered_corrective_402_retries_once() {
    let server = MockServer::start().await;
    let challenge = b64_json(&sample_required());

    Mock::given(method("GET"))
        .and(path("/paid"))
        .and(header(PAYMENT_SIGNATURE, SIGNED))
        .respond_with(
            ResponseTemplate::new(402).insert_header(PAYMENT_REQUIRED, challenge.as_str()),
        )
        .up_to_n_times(1)
        .expect(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/paid"))
        .and(header(PAYMENT_SIGNATURE, SIGNED))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .expect(1)
        .with_priority(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/paid"))
        .respond_with(
            ResponseTemplate::new(402).insert_header(PAYMENT_REQUIRED, challenge.as_str()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let buyer = paid_buyer().with_hook(RecoverHook);
    let client = http_client().with_payments(buyer);
    let response = client
        .get(format!("{}/paid", server.uri()))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        200,
        "corrective 402 with recovered hook must retry once more"
    );
}

#[tokio::test]
async fn retries_402_merges_extension_headers() {
    let server = MockServer::start().await;
    let challenge = b64_json(&sample_required_with_siwx());

    Mock::given(method("GET"))
        .and(path("/paid"))
        .and(header(PAYMENT_SIGNATURE, SIGNED))
        .and(header(SIGN_IN_WITH_X, "cHJvb2Y="))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/paid"))
        .respond_with(
            ResponseTemplate::new(402).insert_header(PAYMENT_REQUIRED, challenge.as_str()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let buyer = paid_buyer().with_extension(SignInHeaderExt);
    let client = http_client().with_payments(buyer);
    let response = client
        .get(format!("{}/paid", server.uri()))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        200,
        "402 retry must merge SIGN-IN-WITH-X onto the paid request"
    );
}

#[tokio::test]
async fn omits_extension_headers_when_not_declared() {
    let server = MockServer::start().await;
    let challenge = b64_json(&sample_required());

    Mock::given(method("GET"))
        .and(path("/paid"))
        .and(header(PAYMENT_SIGNATURE, SIGNED))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/paid"))
        .respond_with(
            ResponseTemplate::new(402).insert_header(PAYMENT_REQUIRED, challenge.as_str()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let buyer = paid_buyer().with_extension(SignInHeaderExt);
    let client = http_client().with_payments(buyer);
    let response = client
        .get(format!("{}/paid", server.uri()))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let received = server.received_requests().await.unwrap();
    let paid = received
        .iter()
        .find(|req| req.headers.get(PAYMENT_SIGNATURE).is_some())
        .expect("paid retry");
    assert!(
        paid.headers.get(SIGN_IN_WITH_X).is_none(),
        "undeclared SIWX must not set SIGN-IN-WITH-X"
    );
}

fn siwx_required_for(origin: &SiwxOrigin, path: &str, resource_url: &str) -> PaymentRequired {
    let entry = SiwxExtension::new(origin.clone())
        .with_chain(SiwxChain::eip191("eip155:8453"))
        .challenge_now(path)
        .unwrap();
    let mut required = sample_required();
    required.resource.url = resource_url.into();
    required.extensions.insert(SIWX_KEY, entry);
    required
}

async fn paid_retry_has_siwx(server: &MockServer) -> bool {
    server.received_requests().await.unwrap().iter().any(|req| {
        req.headers.get(PAYMENT_SIGNATURE).is_some() && req.headers.get(SIGN_IN_WITH_X).is_some()
    })
}

async fn mount_start_redirect_to_paid_402(server: &MockServer, challenge: &str) {
    let paid = format!("{}/paid", server.uri());
    Mock::given(method("GET"))
        .and(path("/start"))
        .and(header(PAYMENT_SIGNATURE, SIGNED))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .expect(1)
        .with_priority(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", paid.as_str()))
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/paid"))
        .respond_with(ResponseTemplate::new(402).insert_header(PAYMENT_REQUIRED, challenge))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn middleware_siwx_binds_response_url_not_resource_url() {
    let server = MockServer::start().await;
    let origin = SiwxOrigin::parse(server.uri().as_str()).unwrap();
    let paid = format!("{}/paid", server.uri());
    let challenge = b64_json(&siwx_required_for(
        &origin,
        "/paid",
        "https://evil.example/paid",
    ));
    mount_start_redirect_to_paid_402(&server, &challenge).await;

    let urls = Arc::new(Mutex::new(Vec::new()));
    let buyer = paid_buyer().with_extension(RecordingSiwx::new(Arc::clone(&urls)));
    let client = http_client().with_payments(buyer);
    let response = client
        .get(format!("{}/start", server.uri()))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(*urls.lock().unwrap(), vec![paid]);
    assert!(
        paid_retry_has_siwx(&server).await,
        "challenge matching the redirected 402 URL must sign even when resource.url is another origin"
    );
}

#[tokio::test]
async fn middleware_siwx_omits_header_when_402_origin_mismatches() {
    let server = MockServer::start().await;
    let evil = SiwxOrigin::parse("https://evil.example").unwrap();
    let paid = format!("{}/paid", server.uri());
    let challenge = b64_json(&siwx_required_for(&evil, "/paid", paid.as_str()));
    mount_start_redirect_to_paid_402(&server, &challenge).await;

    let urls = Arc::new(Mutex::new(Vec::new()));
    let buyer = paid_buyer().with_extension(RecordingSiwx::new(Arc::clone(&urls)));
    let client = http_client().with_payments(buyer);
    let response = client
        .get(format!("{}/start", server.uri()))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(*urls.lock().unwrap(), vec![paid]);
    assert!(
        !paid_retry_has_siwx(&server).await,
        "challenge origin mismatch against the 402 URL must not set SIGN-IN-WITH-X"
    );
}

#[tokio::test]
async fn make_payment_headers_passes_response_url() {
    let server = MockServer::start().await;
    let origin = SiwxOrigin::parse(server.uri().as_str()).unwrap();
    let paid = format!("{}/paid", server.uri());
    let challenge = b64_json(&siwx_required_for(
        &origin,
        "/paid",
        "https://evil.example/paid",
    ));
    Mock::given(method("GET"))
        .and(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", paid.as_str()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/paid"))
        .respond_with(
            ResponseTemplate::new(402).insert_header(PAYMENT_REQUIRED, challenge.as_str()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let urls = Arc::new(Mutex::new(Vec::new()));
    let buyer = paid_buyer().with_extension(RecordingSiwx::new(Arc::clone(&urls)));
    let res = http_client()
        .get(format!("{}/start", server.uri()))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 402);
    assert_eq!(res.url().as_str(), paid.as_str());
    let headers = buyer.make_payment_headers(res).await.unwrap();
    assert!(headers.get(SIGN_IN_WITH_X).is_some());
    assert_eq!(*urls.lock().unwrap(), vec![paid]);
}

#[tokio::test]
async fn middleware_recovery_uses_paid_attempt_url() {
    let server = MockServer::start().await;
    let challenge = b64_json(&sample_required_with_siwx());
    let other = format!("{}/other", server.uri());
    let urls = Arc::new(Mutex::new(Vec::new()));

    Mock::given(method("GET"))
        .and(path("/paid"))
        .and(header(PAYMENT_SIGNATURE, SIGNED))
        .respond_with(ResponseTemplate::new(302).insert_header("location", other.as_str()))
        .up_to_n_times(1)
        .expect(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/paid"))
        .and(header(PAYMENT_SIGNATURE, SIGNED))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .expect(1)
        .with_priority(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/other"))
        .respond_with(
            ResponseTemplate::new(402).insert_header(PAYMENT_REQUIRED, challenge.as_str()),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/paid"))
        .respond_with(
            ResponseTemplate::new(402).insert_header(PAYMENT_REQUIRED, challenge.as_str()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let buyer = paid_buyer()
        .with_hook(RecoverHook)
        .with_extension(CaptureUrlExt {
            urls: Arc::clone(&urls),
        });
    let client = http_client().with_payments(buyer);
    let response = client
        .get(format!("{}/paid", server.uri()))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        *urls.lock().unwrap(),
        vec![format!("{}/paid", server.uri()), other]
    );
}
