//! HTTP facilitator client: Transport three-state, EXTENSION-RESPONSES, 429 retry.

#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::indexing_slicing,
    clippy::shadow_unrelated,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "idiomatic test-code patterns"
)]

use std::time::Duration;

use r402_facilitator::{
    Facilitator, FacilitatorClient, FacilitatorClientError, compute_retry_delay,
};
use r402_protocol::error::{ErrorReason, FacilitatorError, FacilitatorTransportKind};
use r402_protocol::payment::{
    Base64Bytes, Extensions, SettleRequest, SettleResponse, SupportedPaymentKind,
    SupportedResponse, VerifyRequest, VerifyResponse,
};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn test_client(uri: &str) -> FacilitatorClient {
    FacilitatorClient::try_new(uri.parse().unwrap()).unwrap()
}

fn dummy_verify() -> VerifyRequest {
    serde_json::json!({}).into()
}

fn dummy_settle() -> SettleRequest {
    serde_json::json!({}).into()
}

fn supported_body() -> SupportedResponse {
    SupportedResponse::new().with_kinds(vec![SupportedPaymentKind::new(2, "exact", "eip155:8453")])
}

fn encode_extension_header(value: &serde_json::Value) -> String {
    let json = serde_json::to_vec(value).unwrap();
    String::from_utf8(Base64Bytes::encode(json).0).unwrap()
}

fn assert_transport(err: &FacilitatorError, expected: FacilitatorTransportKind) {
    assert!(
        err.as_payment_problem().is_none(),
        "Transport must not map to a 402 payment problem: {err:?}"
    );
    match err {
        FacilitatorError::Transport { kind } => {
            assert_eq!(*kind, expected, "unexpected transport kind");
        }
        other => panic!("expected Transport, got {other:?}"),
    }
}

#[test]
fn try_from_str_stores_normalized_base_url() {
    let client = FacilitatorClient::try_from("https://facilitator.example.com")
        .expect("valid facilitator URL");
    assert_eq!(
        client.base_url().as_str(),
        "https://facilitator.example.com/"
    );
    assert_eq!(
        client.verify_url().as_str(),
        "https://facilitator.example.com/verify"
    );
    assert_eq!(
        client.settle_url().as_str(),
        "https://facilitator.example.com/settle"
    );
    assert_eq!(
        client.supported_url().as_str(),
        "https://facilitator.example.com/supported"
    );
}

#[test]
fn try_new_keeps_facilitator_path_prefix() {
    let client = FacilitatorClient::try_new("https://x402.org/facilitator".parse().unwrap())
        .expect("valid URL");
    assert_eq!(client.base_url().as_str(), "https://x402.org/facilitator/");
    assert_eq!(
        client.verify_url().as_str(),
        "https://x402.org/facilitator/verify"
    );
    assert_eq!(
        client.settle_url().as_str(),
        "https://x402.org/facilitator/settle"
    );
    assert_eq!(
        client.supported_url().as_str(),
        "https://x402.org/facilitator/supported"
    );
}

#[test]
fn try_from_str_rejects_invalid_url() {
    let err = FacilitatorClient::try_from("not a url");
    match err {
        Err(FacilitatorClientError::UrlParse { context, .. }) => {
            assert_eq!(context, "Failed to parse base url");
        }
        other => panic!("expected UrlParse, got {other:?}"),
    }
}

#[test]
fn try_new_supported_cache_is_none() {
    let client = FacilitatorClient::try_from("https://facilitator.example.com")
        .expect("valid facilitator URL");
    assert!(
        client.supported_cache().is_none(),
        "try_new must not install a /supported cache"
    );
}

#[test]
fn without_supported_cache_clears_opt_in_cache() {
    let client = FacilitatorClient::try_from("https://facilitator.example.com")
        .expect("valid facilitator URL")
        .with_supported_cache_ttl(Duration::from_mins(10));
    assert!(client.supported_cache().is_some());
    let client = client.without_supported_cache();
    assert!(client.supported_cache().is_none());
}

#[tokio::test]
async fn try_new_does_not_cache_supported() {
    let mock_server = MockServer::start().await;
    let test_response = supported_body();
    Mock::given(method("GET"))
        .and(path("/supported"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&test_response))
        .expect(2)
        .mount(&mock_server)
        .await;

    let client = test_client(&mock_server.uri());
    let result1 = client.supported().await.unwrap();
    let result2 = client.supported().await.unwrap();
    assert_eq!(result1.kinds.len(), 1);
    assert_eq!(result1.kinds[0].scheme, result2.kinds[0].scheme);
}

#[tokio::test]
async fn with_supported_cache_ttl_caches_response() {
    let mock_server = MockServer::start().await;
    let test_response = supported_body();
    Mock::given(method("GET"))
        .and(path("/supported"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&test_response))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = test_client(&mock_server.uri()).with_supported_cache_ttl(Duration::from_mins(10));
    let result1 = client.supported().await.unwrap();
    let result2 = client.supported().await.unwrap();
    assert_eq!(result1.kinds.len(), 1);
    assert_eq!(result2.kinds.len(), 1);
}

#[tokio::test]
async fn supported_cache_shared_across_clones() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/supported"))
        .respond_with(ResponseTemplate::new(200).set_body_json(supported_body()))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = test_client(&mock_server.uri()).with_supported_cache_ttl(Duration::from_mins(10));
    let client2 = client.clone();
    let result1 = client.supported().await.unwrap();
    let result2 = client2.supported().await.unwrap();
    assert_eq!(result1.kinds[0].scheme, result2.kinds[0].scheme);
}

#[tokio::test]
async fn supported_inner_bypasses_cache() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/supported"))
        .respond_with(ResponseTemplate::new(200).set_body_json(supported_body()))
        .expect(2)
        .mount(&mock_server)
        .await;

    let client = test_client(&mock_server.uri()).with_supported_cache_ttl(Duration::from_mins(10));
    let _ = client.supported().await.unwrap();
    let result = client.supported_inner().await.unwrap();
    assert_eq!(result.kinds.len(), 1);
}

#[tokio::test]
async fn create_auth_headers_returns_path_scoped_values() {
    let client = FacilitatorClient::try_from("https://facilitator.example.com")
        .unwrap()
        .with_auth(|| async {
            Ok(serde_json::json!({
                "verify": { "Authorization": "Bearer verify" },
                "settle": { "Authorization": "Bearer settle" },
                "supported": { "Authorization": "Bearer supported" },
            }))
        });

    let verify = client.create_auth_headers("verify").await.unwrap();
    assert_eq!(verify.get("Authorization").unwrap(), "Bearer verify");
    let settle = client.create_auth_headers("settle").await.unwrap();
    assert_eq!(settle.get("Authorization").unwrap(), "Bearer settle");
    let omitted = client.create_auth_headers("bazaar").await.unwrap();
    assert!(omitted.is_empty());
}

#[tokio::test]
async fn create_auth_headers_looks_flat_rejects_authorization_string() {
    let client = FacilitatorClient::try_from("https://facilitator.example.com")
        .unwrap()
        .with_auth(|| async { Ok(serde_json::json!({ "Authorization": "Bearer token" })) });
    let err = client.create_auth_headers("verify").await.unwrap_err();
    assert!(
        matches!(err, FacilitatorClientError::FlatAuthHeaders),
        "flat Authorization string must error, got {err:?}"
    );
    assert!(err.to_string().contains("keyed by facilitator path"));
}

#[tokio::test]
async fn create_auth_headers_propagates_callback_err() {
    let client = FacilitatorClient::try_from("https://facilitator.example.com")
        .unwrap()
        .with_auth(|| async {
            Err(FacilitatorClientError::Auth(
                "token refresh failed".to_owned(),
            ))
        });
    let err = client.create_auth_headers("verify").await.unwrap_err();
    match err {
        FacilitatorClientError::Auth(message) => assert_eq!(message, "token refresh failed"),
        other => panic!("expected Auth, got {other:?}"),
    }
}

#[tokio::test]
async fn create_auth_headers_rejects_non_string_path_header() {
    let client = FacilitatorClient::try_from("https://facilitator.example.com")
        .unwrap()
        .with_auth(|| async {
            Ok(serde_json::json!({
                "verify": { "Authorization": 123 }
            }))
        });
    let err = client.create_auth_headers("verify").await.unwrap_err();
    match err {
        FacilitatorClientError::InvalidAuthHeader { path, name } => {
            assert_eq!(path, "verify");
            assert_eq!(name, "Authorization");
        }
        other => panic!("expected InvalidAuthHeader, got {other:?}"),
    }
}

#[tokio::test]
async fn verify_sends_path_keyed_auth_headers() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/verify"))
        .and(header("authorization", "Bearer verify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(VerifyResponse::valid("0xpayer")))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = test_client(&mock_server.uri()).with_auth(|| async {
        Ok(serde_json::json!({
            "verify": { "Authorization": "Bearer verify" },
            "settle": { "Authorization": "Bearer settle" },
            "supported": { "Authorization": "Bearer supported" },
        }))
    });

    client
        .verify(&dummy_verify())
        .await
        .expect("verify with path-keyed auth");
}

#[tokio::test]
async fn supported_retries_429_even_when_body_is_supported_json() {
    let mock_server = MockServer::start().await;
    let test_response = supported_body();
    Mock::given(method("GET"))
        .and(path("/supported"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "0")
                .set_body_json(&test_response),
        )
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/supported"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&test_response))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = test_client(&mock_server.uri());
    let result = client
        .supported()
        .await
        .expect("429 with a SupportedResponse-shaped body must still retry");
    assert_eq!(result.kinds.len(), 1);
}

#[tokio::test]
async fn supported_does_not_retry_5xx() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/supported"))
        .respond_with(ResponseTemplate::new(500).set_body_string("unavailable"))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = test_client(&mock_server.uri());
    let err = client.supported().await.unwrap_err();
    assert_transport(&err, FacilitatorTransportKind::HttpStatus { status: 500 });
}

#[tokio::test]
async fn supported_gives_up_after_three_429s() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/supported"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "0")
                .set_body_string("rate limited"),
        )
        .expect(3)
        .mount(&mock_server)
        .await;

    let err = test_client(&mock_server.uri())
        .supported()
        .await
        .unwrap_err();
    assert_transport(&err, FacilitatorTransportKind::HttpStatus { status: 429 });
}

#[tokio::test]
async fn verify_2xx_malformed_body_is_transport() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/verify"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not-json"))
        .mount(&mock_server)
        .await;

    let err = test_client(&mock_server.uri())
        .verify(&dummy_verify())
        .await
        .unwrap_err();
    assert_transport(&err, FacilitatorTransportKind::MalformedSuccessBody);
}

#[tokio::test]
async fn settle_2xx_incomplete_json_is_transport() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/settle"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"success": true})),
        )
        .mount(&mock_server)
        .await;

    let err = test_client(&mock_server.uri())
        .settle(&dummy_settle())
        .await
        .unwrap_err();
    assert_transport(&err, FacilitatorTransportKind::MalformedSuccessBody);
}

#[tokio::test]
async fn verify_4xx_without_verify_json_is_transport() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/verify"))
        .respond_with(ResponseTemplate::new(503).set_body_string("upstream down"))
        .mount(&mock_server)
        .await;

    let err = test_client(&mock_server.uri())
        .verify(&dummy_verify())
        .await
        .unwrap_err();
    assert_transport(&err, FacilitatorTransportKind::HttpStatus { status: 503 });
}

#[tokio::test]
async fn verify_4xx_with_is_valid_json_is_not_transport() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/verify"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(VerifyResponse::invalid(None, ErrorReason::InvalidPayload)),
        )
        .mount(&mock_server)
        .await;

    let response = test_client(&mock_server.uri())
        .verify(&dummy_verify())
        .await
        .expect("well-formed isValid JSON on 4xx is a verify outcome, not Transport");
    assert!(!response.is_valid());
}

#[tokio::test]
async fn verify_4xx_valid_json_is_transport() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/verify"))
        .respond_with(ResponseTemplate::new(400).set_body_json(VerifyResponse::valid("0xpayer")))
        .mount(&mock_server)
        .await;

    let err = test_client(&mock_server.uri())
        .verify(&dummy_verify())
        .await
        .unwrap_err();
    assert_transport(&err, FacilitatorTransportKind::HttpStatus { status: 400 });
}

#[tokio::test]
async fn settle_4xx_with_success_json_is_not_transport() {
    let mock_server = MockServer::start().await;
    let body = SettleResponse::Failure {
        reason: ErrorReason::InsufficientFunds,
        message: None,
        payer: None,
        transaction: "".into(),
        network: "eip155:8453".into(),
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    };
    Mock::given(method("POST"))
        .and(path("/settle"))
        .respond_with(ResponseTemplate::new(400).set_body_json(&body))
        .mount(&mock_server)
        .await;

    let response = test_client(&mock_server.uri())
        .settle(&dummy_settle())
        .await
        .expect("well-formed success JSON on 4xx is a settle outcome, not Transport");
    assert!(!response.is_success());
}

#[tokio::test]
async fn settle_4xx_success_json_is_transport() {
    let mock_server = MockServer::start().await;
    let body = SettleResponse::Success {
        payer: Some("0xabc".into()),
        transaction: "0xabc".into(),
        network: "eip155:8453".into(),
        amount: None,
        extensions: Extensions::new(),
        extension_responses: Extensions::new(),
        extra: None,
    };
    Mock::given(method("POST"))
        .and(path("/settle"))
        .respond_with(ResponseTemplate::new(400).set_body_json(&body))
        .mount(&mock_server)
        .await;

    let err = test_client(&mock_server.uri())
        .settle(&dummy_settle())
        .await
        .unwrap_err();
    assert_transport(&err, FacilitatorTransportKind::HttpStatus { status: 400 });
}

#[tokio::test]
async fn settle_2xx_missing_transaction_is_malformed() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/settle"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "payer": "0xabc",
            "network": "eip155:8453"
        })))
        .mount(&mock_server)
        .await;

    let err = test_client(&mock_server.uri())
        .settle(&dummy_settle())
        .await
        .unwrap_err();
    assert_transport(&err, FacilitatorTransportKind::MalformedSuccessBody);
}

#[tokio::test]
async fn settle_2xx_empty_transaction_with_payer_is_success() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/settle"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "transaction": "",
            "network": "eip155:8453",
            "payer": "0xabc"
        })))
        .mount(&mock_server)
        .await;

    let response = test_client(&mock_server.uri())
        .settle(&dummy_settle())
        .await
        .expect("empty success transaction is legal");
    match response {
        SettleResponse::Success {
            transaction,
            payer,
            network,
            ..
        } => {
            assert!(transaction.is_empty());
            assert_eq!(payer.as_deref(), Some("0xabc"));
            assert_eq!(network, "eip155:8453");
        }
        other => panic!("expected Success, got {other:?}"),
    }
}

#[tokio::test]
async fn settle_2xx_empty_transaction_omitted_payer_is_success() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/settle"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "transaction": "",
            "network": "eip155:8453"
        })))
        .mount(&mock_server)
        .await;

    let response = test_client(&mock_server.uri())
        .settle(&dummy_settle())
        .await
        .expect("omitted payer on empty-tx success is legal");
    match response {
        SettleResponse::Success {
            transaction,
            payer,
            network,
            ..
        } => {
            assert!(transaction.is_empty());
            assert!(payer.is_none());
            assert_eq!(network, "eip155:8453");
        }
        other => panic!("expected Success, got {other:?}"),
    }
}

#[tokio::test]
async fn auth_failure_is_transport_not_payment_problem() {
    let client = FacilitatorClient::try_from("https://facilitator.example.com")
        .unwrap()
        .with_auth(|| async { Err(FacilitatorClientError::Auth("nope".into())) });
    let err = client.verify(&dummy_verify()).await.unwrap_err();
    assert_transport(&err, FacilitatorTransportKind::Io);
}

#[tokio::test]
async fn connect_failure_is_transport_not_payment_problem() {
    let client = FacilitatorClient::try_new("http://127.0.0.1:1/".parse().unwrap()).unwrap();
    let err = client.verify(&dummy_verify()).await.unwrap_err();
    assert!(err.is_transport(), "expected Transport, got {err:?}");
    assert!(
        err.as_payment_problem().is_none(),
        "connect failure must not map to a 402 payment problem"
    );
}

#[tokio::test]
async fn verify_timeout_is_transport() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/verify"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(VerifyResponse::valid("0xpayer"))
                .set_delay(Duration::from_secs(2)),
        )
        .mount(&mock_server)
        .await;

    let err = test_client(&mock_server.uri())
        .with_timeout(Duration::from_millis(20))
        .verify(&dummy_verify())
        .await
        .unwrap_err();
    assert_transport(&err, FacilitatorTransportKind::Timeout);
}

#[tokio::test]
async fn extension_responses_attach_on_settle_without_touching_extensions() {
    let payload = serde_json::json!({"bazaar": {"status": "accepted", "catalogId": "cat-1"}});
    let header = encode_extension_header(&payload);
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/settle"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(SettleResponse::Success {
                    payer: Some("0xabc".into()),
                    transaction: "0xabc".into(),
                    network: "eip155:8453".into(),
                    amount: None,
                    extensions: Extensions::new(),
                    extension_responses: Extensions::new(),
                    extra: None,
                })
                .insert_header("EXTENSION-RESPONSES", header.as_str()),
        )
        .mount(&mock_server)
        .await;

    let response = test_client(&mock_server.uri())
        .settle(&dummy_settle())
        .await
        .unwrap();
    assert!(response.is_success());
    match &response {
        SettleResponse::Success {
            extensions,
            extension_responses,
            ..
        } => {
            assert!(extensions.is_empty());
            assert!(!extension_responses.is_empty());
            let bazaar = extension_responses.get("bazaar").expect("bazaar key");
            assert_eq!(bazaar.to_value()["status"], "accepted");
            assert_eq!(bazaar.to_value()["catalogId"], "cat-1");
        }
        SettleResponse::Failure { .. } => panic!("expected success"),
        _ => panic!("unexpected settle variant"),
    }
}

#[tokio::test]
async fn extension_responses_attach_on_verify_without_touching_extensions() {
    let payload = serde_json::json!({"bazaar": {"status": "accepted"}});
    let header = encode_extension_header(&payload);
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/verify"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(VerifyResponse::valid("0xpayer"))
                .insert_header("EXTENSION-RESPONSES", header.as_str()),
        )
        .mount(&mock_server)
        .await;

    let response = test_client(&mock_server.uri())
        .verify(&dummy_verify())
        .await
        .unwrap();
    assert!(response.is_valid());
    assert!(response.extension_responses().get("bazaar").is_some());
}

#[tokio::test]
async fn body_extensions_stay_independent_from_header() {
    let body_ext = {
        let mut ext = Extensions::new();
        ext.insert(
            "bazaar",
            r402_protocol::payment::ExtensionEntry::raw(serde_json::json!({"status": "from-body"})),
        );
        ext
    };
    let header = encode_extension_header(&serde_json::json!({"bazaar": {"status": "accepted"}}));
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/settle"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(SettleResponse::Success {
                    payer: Some("0xabc".into()),
                    transaction: "0xabc".into(),
                    network: "eip155:8453".into(),
                    amount: None,
                    extensions: body_ext,
                    extension_responses: Extensions::new(),
                    extra: None,
                })
                .insert_header("EXTENSION-RESPONSES", header.as_str()),
        )
        .mount(&mock_server)
        .await;

    let response = test_client(&mock_server.uri())
        .settle(&dummy_settle())
        .await
        .unwrap();
    match response {
        SettleResponse::Success {
            extensions,
            extension_responses,
            ..
        } => {
            assert_eq!(
                extensions.get("bazaar").unwrap().to_value()["status"],
                "from-body"
            );
            assert_eq!(
                extension_responses.get("bazaar").unwrap().to_value()["status"],
                "accepted"
            );
        }
        SettleResponse::Failure { .. } => panic!("expected success"),
        _ => panic!("unexpected settle variant"),
    }
}

#[tokio::test]
async fn malformed_extension_responses_are_ignored() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/settle"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(SettleResponse::Success {
                    payer: Some("0xabc".into()),
                    transaction: "0xabc".into(),
                    network: "eip155:8453".into(),
                    amount: None,
                    extensions: Extensions::new(),
                    extension_responses: Extensions::new(),
                    extra: None,
                })
                .insert_header("EXTENSION-RESPONSES", "not-valid-base64!!!"),
        )
        .mount(&mock_server)
        .await;

    let response = test_client(&mock_server.uri())
        .settle(&dummy_settle())
        .await
        .unwrap();
    assert!(response.is_success());
    assert!(response.extension_responses().is_empty());
}

#[tokio::test]
async fn trait_verify_delegates_to_http() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/verify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(VerifyResponse::valid("0xpayer")))
        .mount(&mock_server)
        .await;
    let client = test_client(&mock_server.uri());
    let response = Facilitator::verify(&client, dummy_verify()).await.unwrap();
    assert!(response.is_valid());
}

#[test]
fn compute_retry_delay_is_reexported() {
    assert_eq!(compute_retry_delay(None, 0), Duration::from_secs(1));
}
