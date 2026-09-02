//! Hook mutation policy for 402 enrichment and settle-response core fields.
//!
//! Fail-closed: a policy violation is an error, not a silent ignore.

use compact_str::CompactString;
use serde_json::{Map, Value};

use crate::error::ErrorReason;
use crate::wire::{PaymentRequirements, SettleResponse};

/// Protocol-reserved `extra` keys. Enrichment must not add or change them.
pub const RESERVED_PAYMENT_FLOW_EXTRA_KEYS: [&str; 2] = ["paymentFlow", "assetTransferMethod"];

/// Hook violated an enrichment mutation policy.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct HookPolicyError(#[doc = "Official-format policy violation message."] pub String);

impl HookPolicyError {
    fn accepts_extension(extension_key: &str, detail: &str) -> Self {
        Self(format!(
            "[x402] extension \"{extension_key}\" violated accepts mutation policy: {detail}"
        ))
    }

    fn accepts_scheme(scheme: &str, detail: &str) -> Self {
        Self(format!(
            "[x402] scheme \"{scheme}\" violated accepts mutation policy: {detail}"
        ))
    }

    fn settle_extension(extension_key: &str, detail: &str) -> Self {
        Self(format!(
            "[x402] extension \"{extension_key}\" violated settlement mutation policy: {detail}"
        ))
    }
}

/// True when a string field is treated as unset and may be filled by enrichment.
#[must_use]
pub fn is_vacant_string_field(value: &str) -> bool {
    value.trim().is_empty()
}

/// Deep snapshot of `accepts` entries before enrichment.
#[must_use]
pub fn snapshot_payment_requirements_list(
    requirements: &[PaymentRequirements],
) -> Vec<PaymentRequirements> {
    requirements.to_vec()
}

/// Extension 402 enrich: vacant `payTo` / `amount` / `asset` may be filled.
///
/// `extra` may gain keys. Reserved payment-flow keys are immutable.
///
/// # Errors
///
/// [`HookPolicyError`] when length, locked fields, extra values, or reserved
/// keys change.
pub fn assert_accepts_allowlisted_after_extension_enrich(
    baseline: &[PaymentRequirements],
    current: &[PaymentRequirements],
    extension_key: &str,
) -> Result<(), HookPolicyError> {
    if baseline.len() != current.len() {
        return Err(HookPolicyError::accepts_extension(
            extension_key,
            &format!(
                "accepts length changed ({} → {})",
                baseline.len(),
                current.len()
            ),
        ));
    }
    for (index, (base, cur)) in baseline.iter().zip(current.iter()).enumerate() {
        assert_extension_row(base, cur, index, extension_key)?;
    }
    Ok(())
}

fn assert_extension_row(
    base: &PaymentRequirements,
    cur: &PaymentRequirements,
    index: usize,
    extension_key: &str,
) -> Result<(), HookPolicyError> {
    if base.scheme != cur.scheme || base.network != cur.network {
        return Err(HookPolicyError::accepts_extension(
            extension_key,
            &format!("scheme/network are immutable (index {index})"),
        ));
    }
    if base.max_timeout_seconds != cur.max_timeout_seconds {
        return Err(HookPolicyError::accepts_extension(
            extension_key,
            &format!("maxTimeoutSeconds is immutable (index {index})"),
        ));
    }
    assert_vacant_or_unchanged(
        "payTo",
        base.pay_to.as_str(),
        cur.pay_to.as_str(),
        index,
        extension_key,
    )?;
    assert_vacant_or_unchanged(
        "amount",
        base.amount.as_str(),
        cur.amount.as_str(),
        index,
        extension_key,
    )?;
    assert_vacant_or_unchanged(
        "asset",
        base.asset.as_str(),
        cur.asset.as_str(),
        index,
        extension_key,
    )?;
    assert_extra_keys_unchanged(base.extra.as_ref(), cur.extra.as_ref(), index, |detail| {
        HookPolicyError::accepts_extension(extension_key, detail)
    })?;
    assert_reserved_keys_presence(base.extra.as_ref(), cur.extra.as_ref(), index, |detail| {
        HookPolicyError::accepts_extension(extension_key, detail)
    })
}

fn assert_vacant_or_unchanged(
    field: &str,
    baseline: &str,
    current: &str,
    index: usize,
    extension_key: &str,
) -> Result<(), HookPolicyError> {
    if !is_vacant_string_field(baseline) && current != baseline {
        return Err(HookPolicyError::accepts_extension(
            extension_key,
            &format!(
                "\"{field}\" may only be set when the resource left it vacant (\"\"); non-vacant values are immutable (index {index})"
            ),
        ));
    }
    Ok(())
}

/// Scheme 402 enrich may only add `extra` keys on matching accepts.
///
/// Payment terms and reserved payment-flow keys are immutable.
///
/// # Errors
///
/// [`HookPolicyError`] when length, terms, extra values, non-matching extra,
/// or reserved keys change.
pub fn assert_accepts_additive_extra_after_scheme_enrich(
    baseline: &[PaymentRequirements],
    current: &[PaymentRequirements],
    scheme: &str,
    network: &str,
) -> Result<(), HookPolicyError> {
    if baseline.len() != current.len() {
        return Err(HookPolicyError::accepts_scheme(
            scheme,
            &format!(
                "accepts length changed ({} → {})",
                baseline.len(),
                current.len()
            ),
        ));
    }
    for (index, (base, cur)) in baseline.iter().zip(current.iter()).enumerate() {
        assert_scheme_row(base, cur, index, scheme, network)?;
    }
    Ok(())
}

fn assert_scheme_row(
    base: &PaymentRequirements,
    cur: &PaymentRequirements,
    index: usize,
    scheme: &str,
    network: &str,
) -> Result<(), HookPolicyError> {
    let is_matching = base.scheme == scheme && base.network.to_string() == network;
    if base.scheme != cur.scheme || base.network != cur.network {
        return Err(HookPolicyError::accepts_scheme(
            scheme,
            &format!("scheme/network are immutable (index {index})"),
        ));
    }
    if base.max_timeout_seconds != cur.max_timeout_seconds
        || base.pay_to != cur.pay_to
        || base.amount != cur.amount
        || base.asset != cur.asset
    {
        return Err(HookPolicyError::accepts_scheme(
            scheme,
            &format!("payment terms are immutable (index {index})"),
        ));
    }
    assert_extra_keys_unchanged(base.extra.as_ref(), cur.extra.as_ref(), index, |detail| {
        HookPolicyError::accepts_scheme(scheme, detail)
    })?;
    if !is_matching && extra_len(cur.extra.as_ref()) != extra_len(base.extra.as_ref()) {
        return Err(HookPolicyError::accepts_scheme(
            scheme,
            &format!("only matching accepts may receive new extra fields (index {index})"),
        ));
    }
    assert_reserved_keys_presence(base.extra.as_ref(), cur.extra.as_ref(), index, |detail| {
        HookPolicyError::accepts_scheme(scheme, detail)
    })
}

fn assert_extra_keys_unchanged(
    baseline: Option<&Value>,
    current: Option<&Value>,
    index: usize,
    error: impl Fn(&str) -> HookPolicyError,
) -> Result<(), HookPolicyError> {
    let Some(base_map) = extra_object(baseline) else {
        return Ok(());
    };
    let current_map = extra_object(current);
    for (key, base_value) in base_map {
        let Some(current_value) = current_map.and_then(|map| map.get(key)) else {
            return Err(error(&format!(
                "extra[\"{key}\"] was removed (index {index})"
            )));
        };
        if current_value != base_value {
            return Err(error(&format!(
                "extra[\"{key}\"] may not be changed (index {index})"
            )));
        }
    }
    Ok(())
}

fn assert_reserved_keys_presence(
    baseline: Option<&Value>,
    current: Option<&Value>,
    index: usize,
    error: impl Fn(&str) -> HookPolicyError,
) -> Result<(), HookPolicyError> {
    for key in RESERVED_PAYMENT_FLOW_EXTRA_KEYS {
        if extra_has_key(baseline, key) != extra_has_key(current, key) {
            return Err(error(&format!(
                "extra[\"{key}\"] is protocol-reserved and immutable during enrichment (index {index})"
            )));
        }
    }
    Ok(())
}

fn extra_object(extra: Option<&Value>) -> Option<&Map<String, Value>> {
    extra.and_then(Value::as_object)
}

fn extra_has_key(extra: Option<&Value>, key: &str) -> bool {
    extra_object(extra).is_some_and(|map| map.contains_key(key))
}

fn extra_len(extra: Option<&Value>) -> usize {
    extra_object(extra).map_or(0, Map::len)
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
            payer: Some(payer.clone()),
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

/// Ensures scheme settlement-payload enrichment only adds new keys.
///
/// # Errors
///
/// [`HookPolicyError`] when `enrichment` collides with an existing payload key.
pub fn assert_additive_payload_enrichment(
    payload: &Map<String, Value>,
    enrichment: &Map<String, Value>,
    caller_label: &str,
) -> Result<(), HookPolicyError> {
    for key in enrichment.keys() {
        if payload.contains_key(key) {
            return Err(HookPolicyError(format!(
                "[x402] {caller_label} violated settlement payload enrichment policy: \"{key}\" already exists on the client payload"
            )));
        }
    }
    Ok(())
}

/// Ensures scheme response enrichment only adds new `extra` fields, including nested objects.
///
/// # Errors
///
/// [`HookPolicyError`] when enrichment would overwrite an existing field.
pub fn assert_additive_settlement_extra(
    extra: &Map<String, Value>,
    enrichment: &Map<String, Value>,
    caller_label: &str,
) -> Result<(), HookPolicyError> {
    assert_additive_record(extra, enrichment, caller_label, "extra")
}

/// Merges server-owned settlement response fields after additive policy validation.
#[must_use]
pub fn merge_additive_settlement_extra(
    extra: &Map<String, Value>,
    enrichment: &Map<String, Value>,
) -> Map<String, Value> {
    merge_additive_record(extra, enrichment)
}

fn is_plain_record(value: &Value) -> Option<&Map<String, Value>> {
    value.as_object()
}

fn assert_additive_record(
    target: &Map<String, Value>,
    enrichment: &Map<String, Value>,
    caller_label: &str,
    path: &str,
) -> Result<(), HookPolicyError> {
    for (key, enrichment_value) in enrichment {
        let next_path = format!("{path}[\"{key}\"]");
        let Some(target_value) = target.get(key) else {
            continue;
        };
        if let (Some(target_map), Some(enrichment_map)) = (
            is_plain_record(target_value),
            is_plain_record(enrichment_value),
        ) {
            assert_additive_record(target_map, enrichment_map, caller_label, &next_path)?;
            continue;
        }
        return Err(HookPolicyError(format!(
            "[x402] {caller_label} violated settlement response enrichment policy: {next_path} already exists on the settlement result"
        )));
    }
    Ok(())
}

fn merge_additive_record(
    target: &Map<String, Value>,
    enrichment: &Map<String, Value>,
) -> Map<String, Value> {
    let mut merged = target.clone();
    for (key, enrichment_value) in enrichment {
        match (
            merged.get(key).and_then(is_plain_record),
            is_plain_record(enrichment_value),
        ) {
            (Some(target_map), Some(enrichment_map)) => {
                let nested = merge_additive_record(target_map, enrichment_map);
                merged.insert(key.clone(), Value::Object(nested));
            }
            _ => {
                merged.insert(key.clone(), enrichment_value.clone());
            }
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::wire::Extensions;

    fn sample_req() -> PaymentRequirements {
        PaymentRequirements::new(
            "test-scheme".into(),
            "test:network".parse().unwrap(),
            "1000000".into(),
            "test_recipient".into(),
            "TEST_ASSET".into(),
            300,
        )
        .with_extra(json!({}))
    }

    fn success_settle() -> SettleResponse {
        SettleResponse::Success {
            payer: "0xpayer".into(),
            transaction: "0xtx".into(),
            network: "eip155:8453".into(),
            amount: None,
            extensions: Extensions::new(),
            extra: None,
        }
    }

    #[test]
    fn vacant_string_treats_empty_and_whitespace_as_vacant() {
        assert!(is_vacant_string_field(""));
        assert!(is_vacant_string_field("   "));
        assert!(!is_vacant_string_field("0xabc"));
    }

    #[test]
    fn extension_enrich_allows_filling_vacant_pay_to_amount_asset() {
        let baseline = snapshot_payment_requirements_list(&[sample_req()
            .with_extra(json!({}))
            .pipe_pay_to("")
            .pipe_amount("")
            .pipe_asset("")]);
        let mut current = snapshot_payment_requirements_list(&baseline);
        current[0].pay_to = "0xnew".into();
        current[0].amount = "1".into();
        current[0].asset = "USDC".into();
        assert!(
            assert_accepts_allowlisted_after_extension_enrich(&baseline, &current, "ext").is_ok()
        );
    }

    #[test]
    fn extension_enrich_rejects_scheme_change() {
        let baseline = snapshot_payment_requirements_list(&[sample_req()]);
        let mut current = snapshot_payment_requirements_list(&baseline);
        current[0].scheme = "other".into();
        let err = assert_accepts_allowlisted_after_extension_enrich(&baseline, &current, "ext")
            .unwrap_err();
        assert!(err.to_string().contains("scheme/network"), "{err}");
    }

    #[test]
    fn extension_enrich_rejects_non_vacant_amount_change() {
        let baseline = snapshot_payment_requirements_list(&[PaymentRequirements::new(
            "test-scheme".into(),
            "test:network".parse().unwrap(),
            "1000".into(),
            "test_recipient".into(),
            "TEST_ASSET".into(),
            300,
        )
        .with_extra(json!({}))]);
        let mut current = snapshot_payment_requirements_list(&baseline);
        current[0].amount = "999".into();
        let err = assert_accepts_allowlisted_after_extension_enrich(&baseline, &current, "ext")
            .unwrap_err();
        assert!(err.to_string().contains("amount"), "{err}");
        assert!(err.to_string().contains("vacant"), "{err}");
    }

    #[test]
    fn extension_enrich_rejects_removed_extra_key() {
        let baseline =
            snapshot_payment_requirements_list(&[sample_req().with_extra(json!({"k": 1}))]);
        let mut current = snapshot_payment_requirements_list(&baseline);
        current[0].extra = Some(json!({}));
        let err = assert_accepts_allowlisted_after_extension_enrich(&baseline, &current, "ext")
            .unwrap_err();
        assert!(err.to_string().contains("extra[\"k\"]"), "{err}");
    }

    #[test]
    fn extension_enrich_rejects_changed_extra_value() {
        let baseline =
            snapshot_payment_requirements_list(&[sample_req().with_extra(json!({"k": 1}))]);
        let mut current = snapshot_payment_requirements_list(&baseline);
        current[0].extra = Some(json!({"k": 2}));
        let err = assert_accepts_allowlisted_after_extension_enrich(&baseline, &current, "ext")
            .unwrap_err();
        assert!(err.to_string().contains("extra[\"k\"]"), "{err}");
    }

    #[test]
    fn extension_enrich_allows_adding_extra_keys() {
        let baseline =
            snapshot_payment_requirements_list(&[sample_req().with_extra(json!({"k": 1}))]);
        let mut current = snapshot_payment_requirements_list(&baseline);
        current[0].extra = Some(json!({"k": 1, "newKey": true}));
        assert!(
            assert_accepts_allowlisted_after_extension_enrich(&baseline, &current, "ext").is_ok()
        );
    }

    #[test]
    fn extension_enrich_detects_nested_extra_mutation() {
        let baseline = snapshot_payment_requirements_list(&[
            sample_req().with_extra(json!({"nested": {"b": "c"}}))
        ]);
        let mut current = snapshot_payment_requirements_list(&baseline);
        if let Some(Value::Object(map)) = current[0].extra.as_mut()
            && let Some(Value::Object(nested)) = map.get_mut("nested")
        {
            nested.insert("b".into(), json!("mutated"));
        }
        let err = assert_accepts_allowlisted_after_extension_enrich(&baseline, &current, "ext")
            .unwrap_err();
        assert!(err.to_string().contains("extra[\"nested\"]"), "{err}");
    }

    #[test]
    fn extension_enrich_rejects_injected_payment_flow() {
        let baseline = snapshot_payment_requirements_list(&[
            sample_req().with_extra(json!({"schemeField": "x"}))
        ]);
        let mut current = snapshot_payment_requirements_list(&baseline);
        current[0].extra = Some(json!({"schemeField": "x", "paymentFlow": "upfront"}));
        let err = assert_accepts_allowlisted_after_extension_enrich(&baseline, &current, "ext")
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("extra[\"paymentFlow\"]"), "{msg}");
        assert!(msg.contains("protocol-reserved"), "{msg}");
    }

    #[test]
    fn extension_enrich_rejects_injected_asset_transfer_method() {
        let baseline = snapshot_payment_requirements_list(&[
            sample_req().with_extra(json!({"schemeField": "x"}))
        ]);
        let mut current = snapshot_payment_requirements_list(&baseline);
        current[0].extra = Some(json!({"schemeField": "x", "assetTransferMethod": "permit2"}));
        let err = assert_accepts_allowlisted_after_extension_enrich(&baseline, &current, "ext")
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("extra[\"assetTransferMethod\"]"), "{msg}");
        assert!(msg.contains("protocol-reserved"), "{msg}");
    }

    #[test]
    fn extension_enrich_rejects_changed_payment_flow() {
        let baseline = snapshot_payment_requirements_list(&[
            sample_req().with_extra(json!({"paymentFlow": "authorization"}))
        ]);
        let mut current = snapshot_payment_requirements_list(&baseline);
        current[0].extra = Some(json!({"paymentFlow": "upfront"}));
        let err = assert_accepts_allowlisted_after_extension_enrich(&baseline, &current, "ext")
            .unwrap_err();
        assert!(err.to_string().contains("extra[\"paymentFlow\"]"), "{err}");
    }

    #[test]
    fn extension_enrich_rejects_changed_asset_transfer_method() {
        let baseline = snapshot_payment_requirements_list(&[
            sample_req().with_extra(json!({"assetTransferMethod": "eip3009"}))
        ]);
        let mut current = snapshot_payment_requirements_list(&baseline);
        current[0].extra = Some(json!({"assetTransferMethod": "permit2"}));
        let err = assert_accepts_allowlisted_after_extension_enrich(&baseline, &current, "ext")
            .unwrap_err();
        assert!(
            err.to_string().contains("extra[\"assetTransferMethod\"]"),
            "{err}"
        );
    }

    #[test]
    fn scheme_enrich_rejects_injected_payment_flow_on_matching_accept() {
        let baseline =
            snapshot_payment_requirements_list(&[sample_req().with_extra(json!({"name": "USDC"}))]);
        let mut current = snapshot_payment_requirements_list(&baseline);
        current[0].extra = Some(json!({"name": "USDC", "paymentFlow": "upfront"}));
        let err = assert_accepts_additive_extra_after_scheme_enrich(
            &baseline,
            &current,
            baseline[0].scheme.as_str(),
            &baseline[0].network.to_string(),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("extra[\"paymentFlow\"]"), "{msg}");
        assert!(msg.contains("protocol-reserved"), "{msg}");
    }

    #[test]
    fn scheme_enrich_rejects_injected_asset_transfer_method_on_matching_accept() {
        let baseline =
            snapshot_payment_requirements_list(&[sample_req().with_extra(json!({"name": "USDC"}))]);
        let mut current = snapshot_payment_requirements_list(&baseline);
        current[0].extra = Some(json!({"name": "USDC", "assetTransferMethod": "permit2"}));
        let err = assert_accepts_additive_extra_after_scheme_enrich(
            &baseline,
            &current,
            baseline[0].scheme.as_str(),
            &baseline[0].network.to_string(),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("extra[\"assetTransferMethod\"]"), "{msg}");
        assert!(msg.contains("protocol-reserved"), "{msg}");
    }

    #[test]
    fn scheme_enrich_rejects_changed_payment_flow() {
        let baseline = snapshot_payment_requirements_list(&[
            sample_req().with_extra(json!({"paymentFlow": "escrow"}))
        ]);
        let mut current = snapshot_payment_requirements_list(&baseline);
        current[0].extra = Some(json!({"paymentFlow": "upfront"}));
        let err = assert_accepts_additive_extra_after_scheme_enrich(
            &baseline,
            &current,
            baseline[0].scheme.as_str(),
            &baseline[0].network.to_string(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("extra[\"paymentFlow\"]"), "{err}");
    }

    #[test]
    fn scheme_enrich_rejects_changed_asset_transfer_method() {
        let baseline = snapshot_payment_requirements_list(&[
            sample_req().with_extra(json!({"assetTransferMethod": "eip3009"}))
        ]);
        let mut current = snapshot_payment_requirements_list(&baseline);
        current[0].extra = Some(json!({"assetTransferMethod": "permit2"}));
        let err = assert_accepts_additive_extra_after_scheme_enrich(
            &baseline,
            &current,
            baseline[0].scheme.as_str(),
            &baseline[0].network.to_string(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("extra[\"assetTransferMethod\"]"),
            "{err}"
        );
    }

    #[test]
    fn settle_core_unchanged_allows_extension_map_edits() {
        let mut base = success_settle();
        let snap = snapshot_settle_response_core(&base);
        if let SettleResponse::Success {
            ref mut extensions, ..
        } = base
        {
            extensions.insert("a", crate::wire::ExtensionEntry::info(json!(1)));
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

    #[test]
    fn additive_payload_allows_new_fields() {
        let payload = json_object(&json!({"clientField": "client"}));
        let enrichment = json_object(&json!({"serverField": "server"}));
        assert!(assert_additive_payload_enrichment(&payload, &enrichment, "scheme test").is_ok());
    }

    #[test]
    fn additive_payload_rejects_overwrite() {
        let payload = json_object(&json!({"clientField": "client"}));
        let enrichment = json_object(&json!({"clientField": "server"}));
        let err =
            assert_additive_payload_enrichment(&payload, &enrichment, "scheme test").unwrap_err();
        assert!(err.to_string().contains("clientField"), "{err}");
    }

    #[test]
    fn additive_settlement_extra_allows_new_fields() {
        let extra = json_object(&json!({"facilitatorField": "facilitator"}));
        let enrichment = json_object(&json!({"schemeField": "scheme"}));
        assert!(assert_additive_settlement_extra(&extra, &enrichment, "scheme test").is_ok());
    }

    #[test]
    fn additive_settlement_extra_allows_nested_fields() {
        let extra = json_object(&json!({
            "channelState": {
                "channelId": "0xchannel",
                "balance": "1000",
            }
        }));
        let enrichment = json_object(&json!({
            "channelState": {
                "chargedCumulativeAmount": "200",
            }
        }));
        assert!(assert_additive_settlement_extra(&extra, &enrichment, "scheme test").is_ok());
    }

    #[test]
    fn additive_settlement_extra_rejects_overwrite() {
        let extra = json_object(&json!({"facilitatorField": "facilitator"}));
        let enrichment = json_object(&json!({"facilitatorField": "scheme"}));
        let err = assert_additive_settlement_extra(&extra, &enrichment, "scheme test").unwrap_err();
        assert!(err.to_string().contains("facilitatorField"), "{err}");
    }

    #[test]
    fn additive_settlement_extra_rejects_nested_overwrite() {
        let extra = json_object(&json!({"channelState": {"balance": "1000"}}));
        let enrichment = json_object(&json!({"channelState": {"balance": "2000"}}));
        let err = assert_additive_settlement_extra(&extra, &enrichment, "scheme test").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("channelState"), "{msg}");
        assert!(msg.contains("balance"), "{msg}");
    }

    #[test]
    fn merge_additive_settlement_extra_merges_nested() {
        let extra = json_object(&json!({
            "channelState": {
                "channelId": "0xchannel",
                "balance": "1000",
            }
        }));
        let enrichment = json_object(&json!({
            "chargedAmount": "100",
            "channelState": {
                "chargedCumulativeAmount": "200",
            }
        }));
        let merged = merge_additive_settlement_extra(&extra, &enrichment);
        assert_eq!(
            Value::Object(merged),
            json!({
                "chargedAmount": "100",
                "channelState": {
                    "channelId": "0xchannel",
                    "balance": "1000",
                    "chargedCumulativeAmount": "200",
                }
            })
        );
    }

    fn json_object(value: &Value) -> Map<String, Value> {
        value.as_object().cloned().expect("object")
    }

    trait PipeReq {
        fn pipe_pay_to(self, pay_to: &str) -> Self;
        fn pipe_amount(self, amount: &str) -> Self;
        fn pipe_asset(self, asset: &str) -> Self;
    }

    impl PipeReq for PaymentRequirements {
        fn pipe_pay_to(mut self, pay_to: &str) -> Self {
            self.pay_to = pay_to.into();
            self
        }
        fn pipe_amount(mut self, amount: &str) -> Self {
            self.amount = amount.into();
            self
        }
        fn pipe_asset(mut self, asset: &str) -> Self {
            self.asset = asset.into();
            self
        }
    }
}
