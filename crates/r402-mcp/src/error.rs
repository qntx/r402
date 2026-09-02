//! Errors for MCP × x402 (Go `PaymentRequiredError` + client errors).

use r402_core::wire::PaymentRequired;
use serde_json::Value;

use crate::MCP_PAYMENT_REQUIRED_CODE;

/// Go `PaymentRequiredError` — JSON-RPC style code **402** with payment data.
#[derive(Debug, Clone)]
pub struct PaymentRequiredError {
    /// Always [`MCP_PAYMENT_REQUIRED_CODE`] (402).
    pub code: i32,
    /// Human-readable message.
    pub message: String,
    /// Payment requirements payload.
    pub payment_required: PaymentRequired,
}

impl PaymentRequiredError {
    /// Builds a 402 payment-required error.
    #[must_use]
    pub fn new(message: impl Into<String>, payment_required: PaymentRequired) -> Self {
        Self {
            code: MCP_PAYMENT_REQUIRED_CODE,
            message: message.into(),
            payment_required,
        }
    }
}

impl std::fmt::Display for PaymentRequiredError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PaymentRequiredError {}

/// Failure from an MCP `tools/call`.
#[derive(Debug, Clone)]
pub enum McpCallError {
    /// Transport or non-RPC service failure.
    Transport(String),
    /// JSON-RPC error from `rmcp` `ServiceError::McpError`.
    Rpc {
        /// JSON-RPC error code.
        code: i32,
        /// JSON-RPC error message.
        message: String,
        /// JSON-RPC `data` member.
        data: Option<Value>,
    },
}

impl std::fmt::Display for McpCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(msg) => f.write_str(msg),
            Self::Rpc {
                code,
                message,
                data,
            } => {
                write!(f, "{code}: {message}")?;
                if let Some(data) = data {
                    write!(f, "({data})")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for McpCallError {}

/// Maps [`rmcp::ServiceError`] from an rmcp tool call into [`McpCallError`].
#[cfg(feature = "client")]
#[must_use]
pub fn mcp_call_error_from_rmcp(err: rmcp::ServiceError) -> McpCallError {
    match err {
        rmcp::ServiceError::McpError(data) => McpCallError::Rpc {
            code: data.code.0,
            message: data.message.into_owned(),
            data: data.data,
        },
        other => McpCallError::Transport(other.to_string()),
    }
}

/// Client orchestration errors.
#[derive(Debug, thiserror::Error)]
pub enum McpClientError {
    /// Underlying MCP transport / tool call failed.
    #[error("mcp transport: {0}")]
    Transport(String),
    /// Building a payment payload failed.
    #[error("payment creation: {0}")]
    Payment(String),
    /// Server still required payment after a signed attempt.
    ///
    /// Carries the corrective challenge (when present) and whether
    /// `on_payment_response` requested recovery so callers can retry.
    #[error("payment still required after attempt")]
    StillRequired {
        /// Corrective `PaymentRequired` from the tool result or a JSON-RPC `402` / `-32042` error, if parsed.
        payment_required: Option<Box<PaymentRequired>>,
        /// `true` when payment-response hooks signalled recovery.
        recovery_requested: bool,
    },
    /// Auto-payment disabled or user denied — includes 402 data when available.
    #[error(transparent)]
    PaymentRequired(#[from] Box<PaymentRequiredError>),
}

impl From<PaymentRequiredError> for McpClientError {
    fn from(value: PaymentRequiredError) -> Self {
        Self::PaymentRequired(Box::new(value))
    }
}

impl From<McpCallError> for McpClientError {
    fn from(value: McpCallError) -> Self {
        Self::Transport(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn rpc_display_includes_data() {
        let data = json!({"mode": "url"});
        let err = McpCallError::Rpc {
            code: -32042,
            message: "elicit".into(),
            data: Some(data.clone()),
        };
        assert_eq!(
            err.to_string(),
            format!("-32042: elicit({data})"),
            "Display keeps JSON-RPC data like rmcp ErrorData"
        );
        match McpClientError::from(err) {
            McpClientError::Transport(s) => {
                assert_eq!(
                    s,
                    format!("-32042: elicit({data})"),
                    "From keeps JSON-RPC data"
                );
            }
            other => panic!("expected Transport, got {other:?}"),
        }
    }

    #[test]
    fn rpc_display_omits_missing_data() {
        let err = McpCallError::Rpc {
            code: -32603,
            message: "internal".into(),
            data: None,
        };
        assert_eq!(
            err.to_string(),
            "-32603: internal",
            "no empty parentheses when data is absent"
        );
    }
}
