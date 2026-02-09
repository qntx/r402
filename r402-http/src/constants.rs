//! HTTP-specific constants for the x402 protocol.
//!
//! Corresponds to Python SDK's `http/constants.py`.

/// HTTP header for V2 payment signatures (client → server).
pub const PAYMENT_SIGNATURE_HEADER: &str = "PAYMENT-SIGNATURE";

/// HTTP header for 402 payment requirements (server → client).
pub const PAYMENT_REQUIRED_HEADER: &str = "PAYMENT-REQUIRED";

/// HTTP header for settlement results (server → client).
pub const PAYMENT_RESPONSE_HEADER: &str = "PAYMENT-RESPONSE";

/// V1 legacy header for payment payload (client → server).
pub const X_PAYMENT_HEADER: &str = "X-PAYMENT";

/// V1 legacy header for settlement results.
pub const X_PAYMENT_RESPONSE_HEADER: &str = "X-PAYMENT-RESPONSE";

/// CORS header name for exposing custom headers.
pub const ACCESS_CONTROL_EXPOSE_HEADERS: &str = "Access-Control-Expose-Headers";

/// HTTP 402 Payment Required status code.
pub const HTTP_STATUS_PAYMENT_REQUIRED: u16 = 402;

/// Default CDP facilitator service URL.
pub const DEFAULT_FACILITATOR_URL: &str = "https://x402.org/facilitator";
