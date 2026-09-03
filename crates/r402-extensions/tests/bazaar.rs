//! Advertise-only bazaar discovery declarations.

#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::panic,
    reason = "idiomatic test-code patterns"
)]

use r402_extensions::{BazaarBodyType, BazaarExtension};
use r402_protocol::extension::{AdvertiseContext, Extension};
use serde_json::json;

#[test]
fn advertise_http_query_shape() {
    let ext = BazaarExtension::http_query(json!({"city": "San Francisco"}))
        .with_output(json!({"weather": "foggy"}));
    assert_eq!(ext.id(), "bazaar");
    let entry = ext.advertise(&AdvertiseContext::new(None)).unwrap();
    let info = entry.as_info().unwrap();
    assert_eq!(info["input"]["type"], "http");
    assert_eq!(info["input"]["queryParams"]["city"], "San Francisco");
    assert_eq!(info["output"]["example"]["weather"], "foggy");
    assert!(entry.as_schema().is_some());
}

#[test]
fn advertise_http_body_shape() {
    let ext = BazaarExtension::http_body(BazaarBodyType::Json, json!({"query": "example"}));
    let entry = ext.advertise(&AdvertiseContext::new(None)).unwrap();
    let info = entry.as_info().unwrap();
    assert_eq!(info["input"]["type"], "http");
    assert_eq!(info["input"]["bodyType"], "json");
    assert_eq!(info["input"]["body"]["query"], "example");
}

#[test]
fn advertise_mcp_shape() {
    let ext = BazaarExtension::mcp(
        "financial_analysis",
        json!({"type": "object", "properties": {"ticker": {"type": "string"}}}),
    );
    let entry = ext.advertise(&AdvertiseContext::new(None)).unwrap();
    let info = entry.as_info().unwrap();
    assert_eq!(info["input"]["type"], "mcp");
    assert_eq!(info["input"]["toolName"], "financial_analysis");
    assert!(info["input"].get("inputSchema").is_some());
}

#[test]
fn advertise_is_not_legacy_catalog() {
    let ext = BazaarExtension::http_query(json!({}));
    let entry = ext.advertise(&AdvertiseContext::new(None)).unwrap();
    let info = entry.as_info().unwrap();
    assert!(info.get("catalog").is_none());
    assert!(info.get("categories").is_none());
    assert!(info.get("registered").is_none());
}
