//! Client-side MCP auto-pay helper.
//!
//! Host code supplies a single tool-call primitive and a payment signer;
//! [`pay_and_call`] retries once after a payment-required meta response.

use std::future::Future;

use serde_json::Value;

use crate::{MCP_PAYMENT_META_KEY, MCP_PAYMENT_REQUIRED_CODE};

/// A tool call result as seen by the MCP client wrapper.
#[derive(Debug, Clone)]
pub struct McpCallResult {
    /// Whether the server marked the result as an error.
    pub is_error: bool,
    /// Opaque content array / payload.
    pub content: Value,
    /// `_meta` object (may be null).
    pub meta: Value,
}

impl McpCallResult {
    /// Returns true when the result is an x402 payment-required challenge.
    #[must_use]
    pub fn is_payment_required(&self) -> bool {
        if !self.is_error {
            return false;
        }
        if self.meta.get(MCP_PAYMENT_META_KEY).is_some() {
            return true;
        }
        // Fallback: text content carries the official code string.
        self.content
            .to_string()
            .contains(MCP_PAYMENT_REQUIRED_CODE)
    }

    /// Extracts the payment-required JSON from `_meta`, if present.
    #[must_use]
    pub fn payment_required_value(&self) -> Option<&Value> {
        self.meta.get(MCP_PAYMENT_META_KEY)
    }
}

/// Errors from [`pay_and_call`].
#[derive(Debug, thiserror::Error)]
pub enum PayAndCallError {
    /// Underlying MCP transport / tool call failed.
    #[error("mcp tool call failed: {0}")]
    Transport(String),
    /// Payment creation failed.
    #[error("payment creation failed: {0}")]
    Payment(String),
    /// Server still rejected after payment was attached.
    #[error("payment rejected after retry")]
    StillRequired,
}

/// Calls `call` once; on payment-required, builds a payment via `sign_payment`
/// and retries with the payment JSON in `_meta`.
///
/// # Errors
///
/// Returns [`PayAndCallError`] on transport, signing, or final rejection.
pub async fn pay_and_call<C, S, FutC, FutS>(
    mut call: C,
    mut sign_payment: S,
) -> Result<McpCallResult, PayAndCallError>
where
    C: FnMut(Option<Value>) -> FutC,
    FutC: Future<Output = Result<McpCallResult, String>>,
    S: FnMut(Value) -> FutS,
    FutS: Future<Output = Result<Value, String>>,
{
    let first = call(None)
        .await
        .map_err(PayAndCallError::Transport)?;
    if !first.is_payment_required() {
        return Ok(first);
    }
    let required = first
        .payment_required_value()
        .cloned()
        .ok_or_else(|| PayAndCallError::Payment("missing payment meta".into()))?;
    let payment = sign_payment(required)
        .await
        .map_err(PayAndCallError::Payment)?;
    let second = call(Some(payment))
        .await
        .map_err(PayAndCallError::Transport)?;
    if second.is_payment_required() {
        return Err(PayAndCallError::StillRequired);
    }
    Ok(second)
}

#[cfg(test)]
#[allow(clippy::excessive_nesting, reason = "async test closures nest by nature")]
mod tests {
    use super::*;
    use serde_json::json;

    fn unpaid_result() -> McpCallResult {
        McpCallResult {
            is_error: true,
            content: json!([{"type":"text","text":"x402_payment_required"}]),
            meta: json!({ "x402/payment": { "x402Version": 2 } }),
        }
    }

    fn paid_result() -> McpCallResult {
        McpCallResult {
            is_error: false,
            content: json!([{"type":"text","text":"ok"}]),
            meta: json!({}),
        }
    }

    #[tokio::test]
    async fn retries_once_with_payment() {
        let mut attempts = 0u8;
        let result = pay_and_call(
            |payment| {
                attempts += 1;
                let paid = payment.is_some();
                async move {
                    Ok(if paid {
                        paid_result()
                    } else {
                        unpaid_result()
                    })
                }
            },
            |_required| async { Ok(json!({"payload": "signed"})) },
        )
        .await
        .unwrap();
        assert!(!result.is_error);
        assert_eq!(attempts, 2);
    }
}
