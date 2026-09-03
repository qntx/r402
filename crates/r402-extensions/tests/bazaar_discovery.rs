//! Official bazaar `GET /discovery/resources` and `GET /discovery/search` client.

#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "idiomatic test-code patterns"
)]

use r402_extensions::{
    BazaarDiscoveryClient, BazaarDiscoveryError, ListDiscoveryResourcesParams,
    SearchDiscoveryResourcesParams, with_bazaar,
};
use r402_facilitator::FacilitatorClient;
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn discovery_client(uri: &str) -> BazaarDiscoveryClient {
    let facilitator = FacilitatorClient::try_from(uri).unwrap();
    with_bazaar(facilitator).unwrap()
}

fn empty_list_body() -> serde_json::Value {
    json!({
        "x402Version": 2,
        "items": [],
        "pagination": { "limit": 20, "offset": 0, "total": 0 }
    })
}

fn empty_search_body() -> serde_json::Value {
    json!({
        "x402Version": 2,
        "resources": []
    })
}

fn catalog_resource(extensions: &serde_json::Value) -> serde_json::Value {
    json!({
        "resource": "https://api.example.com/weather",
        "type": "http",
        "x402Version": 2,
        "accepts": [],
        "lastUpdated": "2024-01-01T00:00:00.000Z",
        "extensions": extensions
    })
}

fn external_bazaar_extensions() -> serde_json::Value {
    json!({
        "bazaar": {
            "schema": { "$ref": "http://127.0.0.1/attacker-schema.json" }
        }
    })
}

#[tokio::test]
async fn list_resources_no_params_hits_discovery_resources() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/discovery/resources"))
        .and(header("content-type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_list_body()))
        .expect(1)
        .mount(&server)
        .await;

    let client = discovery_client(&server.uri());
    let result = client
        .list_resources(&ListDiscoveryResourcesParams::default())
        .await
        .unwrap();

    assert_eq!(result.x402_version, 2);
    assert!(result.items.is_empty());
    assert_eq!(result.pagination.limit, 20);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests[0].url.query(), None);
}

#[tokio::test]
async fn list_resources_sends_all_query_params() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/discovery/resources"))
        .and(query_param("type", "http"))
        .and(query_param(
            "payTo",
            "0x1234567890123456789012345678901234567890",
        ))
        .and(query_param("scheme", "exact"))
        .and(query_param("network", "eip155:8453"))
        .and(query_param("extensions", "bazaar"))
        .and(query_param("limit", "10"))
        .and(query_param("offset", "5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "x402Version": 2,
            "items": [],
            "pagination": { "limit": 10, "offset": 5, "total": 100 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = discovery_client(&server.uri());
    let params = ListDiscoveryResourcesParams {
        resource_type: Some("http".into()),
        pay_to: Some("0x1234567890123456789012345678901234567890".into()),
        scheme: Some("exact".into()),
        network: Some("eip155:8453".into()),
        extensions: Some("bazaar".into()),
        limit: Some(10),
        offset: Some(5),
    };
    let result = client.list_resources(&params).await.unwrap();
    assert_eq!(result.pagination.total, 100);

    let requests = server.received_requests().await.unwrap();
    let query = requests[0].url.query().unwrap();
    assert!(query.contains("network=eip155%3A8453"));
}

#[tokio::test]
async fn list_resources_non_success_uses_official_error_prefix() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/discovery/resources"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Server error"))
        .expect(1)
        .mount(&server)
        .await;

    let client = discovery_client(&server.uri());
    let err = client
        .list_resources(&ListDiscoveryResourcesParams::default())
        .await
        .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("Facilitator listDiscoveryResources failed (500)"),
        "{message}"
    );
    assert!(message.contains("Server error"), "{message}");
}

#[tokio::test]
async fn list_resources_parses_cdp_shaped_item() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/discovery/resources"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "x402Version": 1,
            "items": [{
                "resource": "https://x402.mode.network/ta/indicators",
                "type": "http",
                "x402Version": 1,
                "accepts": [{
                    "scheme": "exact",
                    "network": "eip155:8453",
                    "asset": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
                    "amount": "1000",
                    "payTo": "0xa2477E16dCB42E2AD80f03FE97D7F1a1646cd1c0",
                    "maxTimeoutSeconds": 60,
                    "extra": { "name": "USD Coin", "version": "2" }
                }],
                "lastUpdated": "2024-01-01T00:00:00.000Z",
                "extensions": {}
            }],
            "pagination": { "limit": 1, "offset": 0, "total": 12234 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = discovery_client(&server.uri());
    let result = client
        .list_resources(&ListDiscoveryResourcesParams {
            limit: Some(1),
            ..ListDiscoveryResourcesParams::default()
        })
        .await
        .unwrap();

    assert_eq!(result.x402_version, 1);
    assert_eq!(result.items.len(), 1);
    assert_eq!(
        result.items[0].resource,
        "https://x402.mode.network/ta/indicators"
    );
    assert_eq!(result.items[0].resource_type, "http");
    assert_eq!(result.items[0].x402_version, 1);
    assert_eq!(result.items[0].accepts.len(), 1);
    assert_eq!(result.items[0].last_updated, "2024-01-01T00:00:00.000Z");
    assert_eq!(result.pagination.total, 12234);
}

#[tokio::test]
async fn search_sends_required_query_param() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/discovery/search"))
        .and(query_param("query", "weather APIs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_search_body()))
        .expect(1)
        .mount(&server)
        .await;

    let client = discovery_client(&server.uri());
    let result = client
        .search(&SearchDiscoveryResourcesParams::new("weather APIs"))
        .await
        .unwrap();
    assert_eq!(result.x402_version, 2);
    assert!(result.pagination.is_none());

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests[0].url.query().unwrap(), "query=weather+APIs");
}

#[tokio::test]
async fn search_sends_optional_params() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/discovery/search"))
        .and(query_param("query", "financial data"))
        .and(query_param("type", "http"))
        .and(query_param(
            "payTo",
            "0x1234567890123456789012345678901234567890",
        ))
        .and(query_param("scheme", "exact"))
        .and(query_param("network", "eip155:8453"))
        .and(query_param("extensions", "bazaar"))
        .and(query_param("limit", "10"))
        .and(query_param("cursor", "eyJwYWdlIjoyfQ=="))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "x402Version": 2,
            "resources": [],
            "pagination": { "limit": 10, "cursor": "eyJwYWdlIjoyfQ==" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = discovery_client(&server.uri());
    let params = SearchDiscoveryResourcesParams {
        query: "financial data".into(),
        resource_type: Some("http".into()),
        pay_to: Some("0x1234567890123456789012345678901234567890".into()),
        scheme: Some("exact".into()),
        network: Some("eip155:8453".into()),
        extensions: Some("bazaar".into()),
        limit: Some(10),
        cursor: Some("eyJwYWdlIjoyfQ==".into()),
    };
    let result = client.search(&params).await.unwrap();
    assert_eq!(result.pagination.as_ref().unwrap().limit, 10);
    assert_eq!(
        result.pagination.as_ref().unwrap().cursor.as_deref(),
        Some("eyJwYWdlIjoyfQ==")
    );
}

#[tokio::test]
async fn search_non_success_uses_official_error_prefix() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/discovery/search"))
        .respond_with(ResponseTemplate::new(400).set_body_string("query is required"))
        .expect(1)
        .mount(&server)
        .await;

    let client = discovery_client(&server.uri());
    let err = client
        .search(&SearchDiscoveryResourcesParams::new("test"))
        .await
        .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("Facilitator searchDiscoveryResources failed (400)"),
        "{message}"
    );
}

#[tokio::test]
async fn search_parses_paginated_cursor_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/discovery/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "x402Version": 2,
            "resources": [{
                "resource": "https://api.example.com/weather",
                "type": "http",
                "x402Version": 2,
                "accepts": [],
                "lastUpdated": "2024-01-01T00:00:00.000Z",
                "description": "Weather data",
                "mimeType": "application/json",
                "serviceName": "Weather API",
                "tags": ["weather", "api"]
            }],
            "pagination": { "limit": 10, "cursor": "nextPageToken" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = discovery_client(&server.uri());
    let result = client
        .search(&SearchDiscoveryResourcesParams::new("weather"))
        .await
        .unwrap();
    assert_eq!(result.resources.len(), 1);
    assert_eq!(
        result.resources[0].service_name.as_deref(),
        Some("Weather API")
    );
    assert_eq!(result.pagination.as_ref().unwrap().limit, 10);
    assert_eq!(
        result.pagination.as_ref().unwrap().cursor.as_deref(),
        Some("nextPageToken")
    );
}

#[tokio::test]
async fn search_null_pagination_is_none() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/discovery/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "x402Version": 2,
            "resources": [],
            "pagination": null
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = discovery_client(&server.uri());
    let result = client
        .search(&SearchDiscoveryResourcesParams::new("weather"))
        .await
        .unwrap();
    assert!(result.pagination.is_none());
}

#[tokio::test]
async fn list_resources_sends_bazaar_auth_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/discovery/resources"))
        .and(header("authorization", "Bearer bazaar"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_list_body()))
        .expect(1)
        .mount(&server)
        .await;

    let facilitator = FacilitatorClient::try_from(server.uri().as_str())
        .unwrap()
        .with_auth(|| async {
            Ok(serde_json::json!({
                "bazaar": { "Authorization": "Bearer bazaar" }
            }))
        });
    let client = BazaarDiscoveryClient::new(facilitator).unwrap();
    client
        .list_resources(&ListDiscoveryResourcesParams::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn list_resources_does_not_fail_page_on_open_accepts() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/discovery/resources"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "x402Version": 2,
            "items": [{
                "resource": "https://api.example.com/weather",
                "type": "http",
                "x402Version": 2,
                "accepts": [
                    {"scheme": "exact", "network": "eip155:8453"},
                    {
                        "scheme": "exact",
                        "network": "eip155:8453",
                        "asset": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
                        "amount": "1000",
                        "payTo": "0xa2477E16dCB42E2AD80f03FE97D7F1a1646cd1c0",
                        "maxTimeoutSeconds": 60,
                        "description": "v1 leftover",
                        "resource": "https://api.example.com/weather",
                        "maxAmountRequired": "1000"
                    }
                ],
                "lastUpdated": "2026-01-01T00:00:00Z"
            }],
            "pagination": { "limit": 20, "offset": 0, "total": 1 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = discovery_client(&server.uri());
    let result = client
        .list_resources(&ListDiscoveryResourcesParams::default())
        .await
        .unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].accepts.len(), 2);
    assert_eq!(result.items[0].accepts[0]["network"], "eip155:8453");
    assert_eq!(result.items[0].accepts[1]["maxAmountRequired"], "1000");
}

#[test]
fn discovery_resource_deserializes_optional_extensions() {
    let resource: r402_extensions::DiscoveryResource = serde_json::from_value(json!({
        "resource": "https://api.example.com/endpoint",
        "type": "http",
        "x402Version": 2,
        "accepts": [],
        "lastUpdated": "2024-01-01T00:00:00.000Z",
        "extensions": { "bazaar": { "category": "weather" } }
    }))
    .unwrap();
    assert_eq!(
        resource.extensions.unwrap()["bazaar"]["category"],
        "weather"
    );
}

#[tokio::test]
async fn list_resources_external_schema_ref_fails_page() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/discovery/resources"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "x402Version": 2,
            "items": [catalog_resource(&external_bazaar_extensions())],
            "pagination": { "limit": 20, "offset": 0, "total": 1 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = discovery_client(&server.uri());
    let err = client
        .list_resources(&ListDiscoveryResourcesParams::default())
        .await
        .unwrap_err();
    assert!(matches!(err, BazaarDiscoveryError::ExternalSchemaRef));
    assert_eq!(
        err.to_string(),
        "schema must not contain external $ref/$id references"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn search_external_schema_ref_fails_page() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/discovery/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "x402Version": 2,
            "resources": [catalog_resource(&external_bazaar_extensions())]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = discovery_client(&server.uri());
    let err = client
        .search(&SearchDiscoveryResourcesParams::new("weather"))
        .await
        .unwrap_err();
    assert!(matches!(err, BazaarDiscoveryError::ExternalSchemaRef));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn list_resources_one_bad_item_fails_whole_page() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/discovery/resources"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "x402Version": 2,
            "items": [
                catalog_resource(&json!({ "bazaar": { "category": "weather" } })),
                catalog_resource(&external_bazaar_extensions())
            ],
            "pagination": { "limit": 20, "offset": 0, "total": 2 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = discovery_client(&server.uri());
    let err = client
        .list_resources(&ListDiscoveryResourcesParams::default())
        .await
        .unwrap_err();
    assert!(matches!(err, BazaarDiscoveryError::ExternalSchemaRef));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn list_resources_fragment_ref_is_ok() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/discovery/resources"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "x402Version": 2,
            "items": [catalog_resource(&json!({
                "bazaar": { "schema": { "$ref": "#/definitions/root" } }
            }))],
            "pagination": { "limit": 20, "offset": 0, "total": 1 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = discovery_client(&server.uri());
    let result = client
        .list_resources(&ListDiscoveryResourcesParams::default())
        .await
        .unwrap();
    assert_eq!(result.items.len(), 1);
}

#[tokio::test]
async fn list_resources_schema_url_is_ok() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/discovery/resources"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "x402Version": 2,
            "items": [catalog_resource(&json!({
                "bazaar": {
                    "schema": { "$schema": "https://json-schema.org/draft/2020-12/schema" }
                }
            }))],
            "pagination": { "limit": 20, "offset": 0, "total": 1 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = discovery_client(&server.uri());
    let result = client
        .list_resources(&ListDiscoveryResourcesParams::default())
        .await
        .unwrap();
    assert_eq!(result.items.len(), 1);
}
