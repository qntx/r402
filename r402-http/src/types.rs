//! HTTP-specific types for the x402 payment protocol server middleware.
//!
//! Provides route configuration, payment options, request context, and
//! processing result types used by [`super::server::PaymentGate`].
//!
//! Corresponds to Python SDK's `http/types.py`.

use r402::proto::{PaymentPayload, PaymentRequirements};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A payment option accepted by a protected route.
///
/// Defines a (scheme, network) pair along with price and recipient for a
/// single payment method accepted at an endpoint.
///
/// Corresponds to Python SDK's `PaymentOption`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentOption {
    /// Payment scheme identifier (e.g., `"exact"`).
    pub scheme: String,

    /// Recipient address (e.g., `"0x..."`).
    pub pay_to: String,

    /// Price — a money string (e.g., `"1.50"`) or structured amount.
    pub price: Value,

    /// CAIP-2 network identifier (e.g., `"eip155:8453"`).
    pub network: String,

    /// Maximum payment validity in seconds (defaults to 300).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_timeout_seconds: Option<u64>,

    /// Scheme-specific extra data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

/// Configuration for a payment-protected route.
///
/// Specifies which payment options a route accepts, along with optional
/// metadata for resource description and paywall customisation.
///
/// Corresponds to Python SDK's `RouteConfig`.
#[derive(Debug, Clone)]
pub struct RouteConfig {
    /// Accepted payment options for this route.
    pub accepts: Vec<PaymentOption>,

    /// Override resource URL (defaults to request URL).
    pub resource: Option<String>,

    /// Human-readable description of the resource.
    pub description: Option<String>,

    /// MIME type of the resource.
    pub mime_type: Option<String>,
}

impl RouteConfig {
    /// Creates a new route config with a single payment option.
    #[must_use]
    pub fn single(option: PaymentOption) -> Self {
        Self {
            accepts: vec![option],
            resource: None,
            description: None,
            mime_type: None,
        }
    }

    /// Creates a new route config with multiple payment options.
    #[must_use]
    pub fn multi(options: Vec<PaymentOption>) -> Self {
        Self {
            accepts: options,
            resource: None,
            description: None,
            mime_type: None,
        }
    }

    /// Sets the resource URL override.
    #[must_use]
    pub fn with_resource(mut self, resource: impl Into<String>) -> Self {
        self.resource = Some(resource.into());
        self
    }

    /// Sets the resource description.
    #[must_use]
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Sets the MIME type.
    #[must_use]
    pub fn with_mime_type(mut self, mime: impl Into<String>) -> Self {
        self.mime_type = Some(mime.into());
        self
    }
}

/// Result of processing an HTTP request through the payment gate.
///
/// Corresponds to Python SDK's `HTTPProcessResult`.
#[derive(Debug)]
pub enum ProcessResult {
    /// Route does not require payment — pass through to inner service.
    NoPaymentRequired,

    /// Payment verified successfully.
    PaymentVerified {
        /// The verified payment payload.
        payload: PaymentPayload,
        /// The matching payment requirements.
        requirements: PaymentRequirements,
    },

    /// Payment error — return 402 or error response.
    PaymentError {
        /// HTTP status code (typically 402 or 500).
        status: u16,
        /// Response headers to include.
        headers: Vec<(String, String)>,
        /// JSON response body.
        body: Value,
    },
}

/// Result of settlement processing after a successful response.
///
/// Corresponds to Python SDK's `ProcessSettleResult`.
#[derive(Debug)]
pub struct SettleResult {
    /// Whether settlement succeeded.
    pub success: bool,
    /// Error reason if settlement failed.
    pub error_reason: Option<String>,
    /// Headers to add to the response (e.g., `PAYMENT-RESPONSE`).
    pub headers: Vec<(String, String)>,
    /// Transaction hash/ID.
    pub transaction: Option<String>,
    /// Network identifier.
    pub network: Option<String>,
    /// Payer address.
    pub payer: Option<String>,
}

/// Paywall UI configuration for browser-based 402 responses.
///
/// Corresponds to Python SDK's `PaywallConfig` in `http/types.py`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaywallConfig {
    /// Application name to display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,

    /// URL to application logo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_logo: Option<String>,

    /// Whether this is a testnet deployment.
    #[serde(default)]
    pub testnet: bool,
}

/// A validation error for a route configuration.
///
/// Returned by [`super::server::PaymentGateLayer::validate_routes`] when a
/// payment option references an unregistered scheme or unsupported
/// facilitator combination.
///
/// Corresponds to Python SDK's `RouteValidationError` in `http/types.py`.
#[derive(Debug, Clone)]
pub struct RouteValidationError {
    /// The route pattern (e.g., `"GET /weather"`).
    pub route_pattern: String,
    /// Scheme identifier (e.g., `"exact"`).
    pub scheme: String,
    /// CAIP-2 network identifier.
    pub network: String,
    /// Reason code (`"missing_scheme"` or `"missing_facilitator"`).
    pub reason: String,
    /// Human-readable error message.
    pub message: String,
}

impl std::fmt::Display for RouteValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// A compiled route entry mapping a method + path pattern to its config.
#[derive(Debug, Clone)]
pub(crate) struct CompiledRoute {
    /// HTTP method (uppercase) or `"*"` for any method.
    pub method: String,
    /// Path pattern (e.g., `/weather`, `/api/*`).
    pub path_pattern: String,
    /// Payment configuration for this route.
    pub config: RouteConfig,
}

impl CompiledRoute {
    /// Checks whether this route matches the given method and path.
    pub fn matches(&self, method: &str, path: &str) -> bool {
        // Method match
        if self.method != "*" && !self.method.eq_ignore_ascii_case(method) {
            return false;
        }

        // Path match with simple glob support
        match_path_pattern(&self.path_pattern, path)
    }
}

/// Simple glob-style path matching.
///
/// Supports:
/// - Exact match: `/weather` matches `/weather`
/// - Trailing wildcard: `/api/*` matches `/api/foo` and `/api/foo/bar`
/// - Full wildcard: `*` matches everything
fn match_path_pattern(pattern: &str, path: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    let normalized_path = path.split('?').next().unwrap_or(path);
    let normalized_path = normalized_path.trim_end_matches('/');
    let normalized_pattern = pattern.trim_end_matches('/');

    if normalized_pattern.ends_with("/*") {
        let prefix = &normalized_pattern[..normalized_pattern.len() - 2];
        normalized_path == prefix || normalized_path.starts_with(&format!("{prefix}/"))
    } else {
        normalized_path.eq_ignore_ascii_case(normalized_pattern)
    }
}

/// Parses a route pattern string into method + path.
///
/// Supports formats:
/// - `"GET /weather"` → method=`GET`, path=`/weather`
/// - `"/weather"` → method=`*`, path=`/weather`
/// - `"*"` → method=`*`, path=`*`
pub(crate) fn parse_route_pattern(pattern: &str) -> (String, String) {
    let trimmed = pattern.trim();
    if let Some((method, path)) = trimmed.split_once(char::is_whitespace) {
        (method.to_uppercase(), path.trim().to_owned())
    } else {
        ("*".to_owned(), trimmed.to_owned())
    }
}
