//! Client extension-echo validation (official `validateExtensions`).

use serde_json::Value;

use crate::error::VerificationError;
use crate::payment::{ExtensionEntry, Extensions};

/// Official `SERVER_OWNED_INFO_FIELDS` for `builder-code`.
const BUILDER_CODE: &str = "builder-code";
const SERVER_OWNED_A: &str = "a";
const ADDITIVE_S: &str = "s";
const ADDITIVE_S_MAX: usize = 10;

/// Validates v2 client extension echoes against the 402 advertisement.
///
/// Empty client extensions pass. Server-owned `builder-code.a` must match
/// advertised `info.a` (including when the server did not declare `a`).
/// Additive `builder-code.s` must be a superset of advertised `s` and must
/// not exceed 10 entries.
///
/// # Errors
///
/// [`VerificationError::ExtensionEchoMismatch`] with the failing key.
pub fn validate_extension_echoes(
    advertised: Option<&Extensions>,
    payload: &Extensions,
    dynamic_info_fields: impl Fn(&str) -> &'static [&'static str],
) -> Result<(), VerificationError> {
    if payload.is_empty() {
        return Ok(());
    }
    for (key, echoed_entry) in payload.iter() {
        let advertised_entry = advertised.and_then(|ext| ext.get(key.as_str()));
        let advertised_info = advertised_entry.map(extension_info);
        let echoed_info = extension_info(echoed_entry);

        if advertised_entry.is_some() {
            let dynamic = dynamic_info_fields(key.as_str());
            let advertised_cmp = omit_fields(advertised_info, dynamic);
            let echoed_cmp = omit_fields(Some(echoed_info), dynamic);
            if !info_matches_advertised(key.as_str(), advertised_cmp.as_ref(), echoed_cmp.as_ref())
            {
                return Err(echo_mismatch(key.as_str()));
            }
        }

        if !server_owned_fields_match(key.as_str(), advertised_info, echoed_info) {
            return Err(echo_mismatch(key.as_str()));
        }
    }
    Ok(())
}

fn echo_mismatch(key: &str) -> VerificationError {
    VerificationError::ExtensionEchoMismatch {
        extension_key: key.to_owned(),
    }
}

fn extension_info(entry: &ExtensionEntry) -> &Value {
    match entry {
        ExtensionEntry::Structured { info, .. } => info,
        ExtensionEntry::Raw(value) => value.get("info").unwrap_or(value),
    }
}

fn omit_fields(info: Option<&Value>, fields: &[&str]) -> Option<Value> {
    let info = info?;
    if fields.is_empty() {
        return Some(info.clone());
    }
    let Value::Object(map) = info else {
        return Some(info.clone());
    };
    let mut copy = map.clone();
    for field in fields {
        let _ = copy.remove(*field);
    }
    Some(Value::Object(copy))
}

fn server_owned_fields_match(key: &str, advertised: Option<&Value>, echoed: &Value) -> bool {
    if key != BUILDER_CODE {
        return true;
    }
    let Value::Object(echoed_map) = echoed else {
        return true;
    };
    if !echoed_map.contains_key(SERVER_OWNED_A) {
        return true;
    }
    let Some(echoed_a) = echoed_map.get(SERVER_OWNED_A) else {
        return true;
    };
    if echoed_a.is_null() {
        return true;
    }
    let advertised_a = advertised.and_then(|v| v.get(SERVER_OWNED_A));
    advertised_a == Some(echoed_a)
}

fn info_matches_advertised(key: &str, advertised: Option<&Value>, echoed: Option<&Value>) -> bool {
    object_contains_subset(advertised, echoed, key, None)
}

fn object_contains_subset(
    expected: Option<&Value>,
    actual: Option<&Value>,
    extension_key: &str,
    field_key: Option<&str>,
) -> bool {
    if let Some(field) = field_key
        && extension_key == BUILDER_CODE
        && field == ADDITIVE_S
        && expected.is_some_and(is_array_or_scalar)
        && actual.is_some_and(is_array_or_scalar)
    {
        return additive_array_contains(expected, actual);
    }

    let Some(expected) = expected else {
        return true;
    };
    if !expected.is_object() {
        return actual == Some(expected);
    }
    let Some(actual) = actual else {
        return false;
    };
    let Some(expected_map) = expected.as_object() else {
        return actual == expected;
    };
    let Some(actual_map) = actual.as_object() else {
        return false;
    };
    expected_map.iter().all(|(k, value)| {
        if !actual_map.contains_key(k) {
            return value.is_null();
        }
        object_contains_subset(
            Some(value),
            actual_map.get(k),
            extension_key,
            Some(k.as_str()),
        )
    })
}

fn is_array_or_scalar(value: &Value) -> bool {
    comparable_array(value).is_some()
}

fn comparable_array(value: &Value) -> Option<Vec<&Value>> {
    match value {
        Value::Array(items) => Some(items.iter().collect()),
        Value::Null | Value::Object(_) => None,
        other => Some(vec![other]),
    }
}

fn additive_array_contains(expected: Option<&Value>, actual: Option<&Value>) -> bool {
    let Some(expected) = expected.and_then(comparable_array) else {
        return false;
    };
    let Some(actual) = actual.and_then(comparable_array) else {
        return false;
    };
    if actual.len() > ADDITIVE_S_MAX {
        return false;
    }
    expected
        .iter()
        .all(|exp| actual.iter().any(|act| act == exp))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::payment::ExtensionEntry;

    fn none_dyn(_: &str) -> &'static [&'static str] {
        &[]
    }

    fn advertised(info: Value) -> Extensions {
        let mut ext = Extensions::new();
        ext.insert("builder-code", ExtensionEntry::info(info));
        ext
    }

    fn payload(info: Value) -> Extensions {
        advertised(info)
    }

    #[test]
    fn empty_client_extensions_pass() {
        assert!(validate_extension_echoes(None, &Extensions::new(), none_dyn).is_ok());
    }

    #[test]
    fn forged_a_without_server_declaration_fails() {
        let err = validate_extension_echoes(None, &payload(json!({"a": "forged_app"})), none_dyn)
            .unwrap_err();
        assert!(matches!(
            err,
            VerificationError::ExtensionEchoMismatch { .. }
        ));
    }

    #[test]
    fn client_s_without_declaration_passes() {
        assert!(
            validate_extension_echoes(None, &payload(json!({"s": ["bc_client"]})), none_dyn)
                .is_ok()
        );
    }

    #[test]
    fn mismatched_a_fails() {
        let err = validate_extension_echoes(
            Some(&advertised(json!({"a": "bc_app"}))),
            &payload(json!({"a": "forged_app"})),
            none_dyn,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            VerificationError::ExtensionEchoMismatch { extension_key } if extension_key == "builder-code"
        ));
    }

    #[test]
    fn matching_a_and_superset_s_passes() {
        assert!(
            validate_extension_echoes(
                Some(&advertised(json!({"a": "bc_app", "s": ["bc_server"]}))),
                &payload(json!({"a": "bc_app", "s": ["bc_server", "bc_client"]})),
                none_dyn,
            )
            .is_ok()
        );
    }

    #[test]
    fn dropped_s_entry_fails() {
        assert!(
            validate_extension_echoes(
                Some(&advertised(json!({"s": ["bc_server"]}))),
                &payload(json!({"s": ["bc_client"]})),
                none_dyn,
            )
            .is_err()
        );
    }

    #[test]
    fn padded_s_over_budget_fails() {
        let mut padded = vec!["bc_server".to_owned()];
        padded.extend((0..10).map(|i| format!("bc_fake_{i}")));
        assert!(
            validate_extension_echoes(
                Some(&advertised(json!({"s": ["bc_server"]}))),
                &payload(json!({"s": padded})),
                none_dyn,
            )
            .is_err()
        );
    }

    #[test]
    fn s_at_budget_passes() {
        let mut at_budget = vec!["bc_server".to_owned()];
        at_budget.extend((0..9).map(|i| format!("bc_client_{i}")));
        assert!(
            validate_extension_echoes(
                Some(&advertised(json!({"s": ["bc_server"]}))),
                &payload(json!({"s": at_budget})),
                none_dyn,
            )
            .is_ok()
        );
    }

    #[test]
    fn scalar_s_merges_against_array() {
        assert!(
            validate_extension_echoes(
                Some(&advertised(json!({"s": "bc_server"}))),
                &payload(json!({"s": ["bc_server", "bc_client"]})),
                none_dyn,
            )
            .is_ok()
        );
    }

    #[test]
    fn dynamic_offers_field_is_omitted() {
        fn offers_dyn(key: &str) -> &'static [&'static str] {
            if key == "offer-receipt" {
                &["offers"]
            } else {
                &[]
            }
        }
        let mut advertised = Extensions::new();
        advertised.insert(
            "offer-receipt",
            ExtensionEntry::info(json!({"offers": [1], "kid": "x"})),
        );
        let mut payload = Extensions::new();
        payload.insert(
            "offer-receipt",
            ExtensionEntry::info(json!({"offers": [2], "kid": "x"})),
        );
        assert!(validate_extension_echoes(Some(&advertised), &payload, offers_dyn).is_ok());
    }
}
