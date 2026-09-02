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

use r402_client::{
    ClientHooks, DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner,
    PaymentResponseContext, PaymentResponseResult, SchemeClient,
};
use r402_http::{PAYMENT_REQUIRED, PAYMENT_SIGNATURE, WithPayments, X402Client};
use r402_protocol::payment::{Base64Bytes, PaymentRequired, PaymentRequirements, ResourceInfo};
use r402_protocol::{ChainId, ClientError, SchemeId};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SIGNED: &str = "c2lnbmVk";

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
