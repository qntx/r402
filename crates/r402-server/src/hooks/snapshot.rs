//! Snapshots of payment requirements and facilitator-settled fields.

use compact_str::CompactString;
use r402_protocol::error::ErrorReason;
use r402_protocol::payment::{PaymentRequirements, SettleResponse};

use super::policy::HookPolicyError;

/// Deep snapshot of `accepts` entries before enrichment.
#[must_use]
pub fn snapshot_payment_requirements_list(
    requirements: &[PaymentRequirements],
) -> Vec<PaymentRequirements> {
    requirements.to_vec()
}

/// Facilitator-settled fields that extensions must not rewrite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettleResponseCoreSnapshot {
    /// Whether settlement succeeded.
    pub success: bool,
    /// Broadcast transaction hash (empty when none).
    pub transaction: CompactString,
    /// CAIP-2 network.
    pub network: CompactString,
    /// Settled amount, when present.
    pub amount: Option<CompactString>,
    /// Payer address, when present.
    pub payer: Option<CompactString>,
    /// Failure reason, when settlement failed.
    pub error_reason: Option<ErrorReason>,
    /// Failure message, when settlement failed.
    pub error_message: Option<CompactString>,
}

/// Captures facilitator-settled fields for later comparison.
#[must_use]
pub fn snapshot_settle_response_core(result: &SettleResponse) -> SettleResponseCoreSnapshot {
    match result {
        SettleResponse::Success {
            payer,
            transaction,
            network,
            amount,
            ..
        } => SettleResponseCoreSnapshot {
            success: true,
            transaction: transaction.clone(),
            network: network.clone(),
            amount: amount.clone(),
            payer: payer.clone(),
            error_reason: None,
            error_message: None,
        },
        SettleResponse::Failure {
            reason,
            message,
            payer,
            transaction,
            network,
            ..
        } => SettleResponseCoreSnapshot {
            success: false,
            transaction: transaction.clone(),
            network: network.clone(),
            amount: None,
            payer: payer.clone(),
            error_reason: Some(reason.clone()),
            error_message: message.clone(),
        },
        _ => SettleResponseCoreSnapshot {
            success: result.is_success(),
            transaction: CompactString::default(),
            network: CompactString::default(),
            amount: None,
            payer: None,
            error_reason: None,
            error_message: None,
        },
    }
}

/// Ensures `enrichSettlementResponse` did not rewrite facilitator outcome fields.
///
/// # Errors
///
/// [`HookPolicyError`] when a core field changed.
pub fn assert_settle_response_core_unchanged(
    before: &SettleResponseCoreSnapshot,
    after: &SettleResponse,
    extension_key: &str,
) -> Result<(), HookPolicyError> {
    let after_snap = snapshot_settle_response_core(after);
    assert_settle_field(
        "success",
        before.success == after_snap.success,
        extension_key,
    )?;
    assert_settle_field(
        "transaction",
        before.transaction == after_snap.transaction,
        extension_key,
    )?;
    assert_settle_field(
        "network",
        before.network == after_snap.network,
        extension_key,
    )?;
    assert_settle_field("amount", before.amount == after_snap.amount, extension_key)?;
    assert_settle_field("payer", before.payer == after_snap.payer, extension_key)?;
    assert_settle_field(
        "errorReason",
        before.error_reason == after_snap.error_reason,
        extension_key,
    )?;
    assert_settle_field(
        "errorMessage",
        before.error_message == after_snap.error_message,
        extension_key,
    )
}

fn assert_settle_field(
    field: &str,
    equal: bool,
    extension_key: &str,
) -> Result<(), HookPolicyError> {
    if equal {
        Ok(())
    } else {
        Err(HookPolicyError::settle_extension(
            extension_key,
            &format!("field \"{field}\" is immutable after facilitator settle"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use r402_protocol::payment::Extensions;

    use super::*;

    fn success_settle() -> SettleResponse {
        SettleResponse::Success {
            payer: Some("0xpayer".into()),
            transaction: "0xtx".into(),
            network: "eip155:8453".into(),
            amount: None,
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        }
    }

    #[test]
    fn settle_core_unchanged_allows_extension_map_edits() {
        let mut base = success_settle();
        let snap = snapshot_settle_response_core(&base);
        if let SettleResponse::Success {
            ref mut extensions, ..
        } = base
        {
            extensions.insert(
                "a",
                r402_protocol::payment::ExtensionEntry::info(serde_json::json!(1)),
            );
        }
        assert!(assert_settle_response_core_unchanged(&snap, &base, "ext").is_ok());
    }

    #[test]
    fn settle_core_unchanged_rejects_transaction_change() {
        let mut base = success_settle();
        let snap = snapshot_settle_response_core(&base);
        if let SettleResponse::Success {
            ref mut transaction,
            ..
        } = base
        {
            *transaction = "0xother".into();
        }
        let err = assert_settle_response_core_unchanged(&snap, &base, "ext").unwrap_err();
        assert!(err.to_string().contains("transaction"), "{err}");
    }
}
