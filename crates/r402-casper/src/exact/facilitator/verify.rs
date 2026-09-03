//! Local preflight for Casper exact verify/settle requests.
//!
//! Cryptographic signature verification and on-chain settlement stay with
//! the remote facilitator. These checks reject malformed payments before a
//! network round trip.

use r402_protocol::UnixTimestamp;

use crate::chain::{Address, CasperChainReference, ContractPackageHash};
use crate::exact::CasperExactError;
use crate::exact::payload::{MIN_SETTLEMENT_WINDOW_SECS, v2, validate_payload_shape};

/// Outcome of a successful local validation pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatedPayment {
    /// The payer's addressable key, taken from `authorization.from`.
    pub payer: Address,
    /// The recipient's addressable key.
    pub pay_to: Address,
    /// The CEP-18 contract package hash being transferred.
    pub asset: ContractPackageHash,
    /// The Casper network the payment targets.
    pub chain: CasperChainReference,
}

/// Validates a decoded verify/settle request without contacting the chain.
///
/// # Errors
///
/// Returns the first [`CasperExactError`] encountered.
pub fn validate_request(request: &v2::VerifyRequest) -> Result<ValidatedPayment, CasperExactError> {
    validate_at(request, UnixTimestamp::now().as_secs())
}

/// Validates a request against an explicit wall-clock time.
///
/// # Errors
///
/// See [`validate_request`].
pub fn validate_at(
    request: &v2::VerifyRequest,
    now: u64,
) -> Result<ValidatedPayment, CasperExactError> {
    let requirements = &request.payment_requirements;
    let accepted = &request.payment_payload.accepted;

    assert_requirements_match(accepted, requirements)?;

    let chain = CasperChainReference::try_from(requirements.network.clone())?;

    let asset = requirements
        .asset
        .parse::<ContractPackageHash>()
        .map_err(|_| CasperExactError::InvalidAsset(requirements.asset.to_string()))?;

    let pay_to = requirements
        .pay_to
        .parse::<Address>()
        .map_err(|_| CasperExactError::InvalidPayTo(requirements.pay_to.to_string()))?;

    let extra = requirements
        .extra
        .as_ref()
        .ok_or(CasperExactError::MissingTokenName)?;
    if extra.name.trim().is_empty() {
        return Err(CasperExactError::MissingTokenName);
    }
    if extra.version.trim().is_empty() {
        return Err(CasperExactError::MissingTokenVersion);
    }

    let payload = &request.payment_payload.payload;
    validate_payload_shape(payload)?;

    let authorization = &payload.authorization;

    if authorization.to != pay_to {
        return Err(CasperExactError::PayToMismatch {
            authorization: authorization.to.to_string(),
            requirements: pay_to.to_string(),
        });
    }

    if authorization.value != requirements.amount {
        return Err(CasperExactError::AmountMismatch {
            authorization: authorization.value.to_string(),
            requirements: requirements.amount.to_string(),
        });
    }

    if requirements.amount.is_zero() {
        return Err(CasperExactError::ZeroAmount);
    }

    validate_timing(
        authorization.valid_after.as_secs(),
        authorization.valid_before.as_secs(),
        now,
        requirements.max_timeout_seconds,
    )?;

    Ok(ValidatedPayment {
        payer: authorization.from,
        pay_to,
        asset,
        chain,
    })
}

fn assert_requirements_match(
    accepted: &v2::PaymentRequirements,
    requirements: &v2::PaymentRequirements,
) -> Result<(), CasperExactError> {
    if requirements.matches_payload_accepted(accepted) {
        Ok(())
    } else if accepted.network != requirements.network {
        Err(CasperExactError::NetworkMismatch {
            payload: accepted.network.to_string(),
            requirements: requirements.network.to_string(),
        })
    } else {
        Err(CasperExactError::AcceptedRequirementsMismatch)
    }
}

/// Checks the authorisation validity window against `now`.
///
/// Strict bounds: `validAfter < now < validBefore`. Remaining window must
/// cover `max_timeout_seconds`, never dropping below
/// [`MIN_SETTLEMENT_WINDOW_SECS`].
///
/// # Errors
///
/// Returns [`CasperExactError::NotYetValid`] or [`CasperExactError::Expired`].
pub const fn validate_timing(
    valid_after: u64,
    valid_before: u64,
    now: u64,
    max_timeout_seconds: u64,
) -> Result<(), CasperExactError> {
    if valid_after >= now {
        return Err(CasperExactError::NotYetValid { valid_after, now });
    }
    if valid_before <= now {
        return Err(CasperExactError::Expired { valid_before, now });
    }

    let remaining = valid_before - now;
    let min_remaining = if max_timeout_seconds > MIN_SETTLEMENT_WINDOW_SECS {
        max_timeout_seconds
    } else {
        MIN_SETTLEMENT_WINDOW_SECS
    };
    if remaining < min_remaining {
        return Err(CasperExactError::Expired { valid_before, now });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYER: &str = "0076d080b4e769f0b29c77fc6472d6e425710840c2f46a4506e5544d2ce34f43a3";
    const PUBLIC_KEY: &str = "020376e4f8766e4f33bcc6e20b331b5163f363dc0106063b052ad38afe08637bd867";
    const PAYEE: &str = "00fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321";
    const ASSET: &str = "3d80df21ba4ee4d66a2a1f60c32570dd5685e4b279f6538162a5fd1314847c1e";
    const NOW: u64 = 1_700_000_100;

    fn signature_hex() -> String {
        format!("02{}", "aa".repeat(64))
    }

    fn request_json() -> serde_json::Value {
        let requirements = serde_json::json!({
            "scheme": "exact",
            "network": "casper:casper-test",
            "amount": "1500000000",
            "payTo": PAYEE,
            "maxTimeoutSeconds": 300,
            "asset": ASSET,
            "extra": { "name": "Wrapped CSPR", "version": "1" }
        });
        serde_json::json!({
            "x402Version": 2,
            "paymentPayload": {
                "x402Version": 2,
                "accepted": requirements,
                "payload": {
                    "signature": signature_hex(),
                    "publicKey": PUBLIC_KEY,
                    "authorization": {
                        "from": PAYER,
                        "to": PAYEE,
                        "value": "1500000000",
                        "validAfter": "1700000000",
                        "validBefore": "1700000600",
                        "nonce": "cc".repeat(32),
                    }
                }
            },
            "paymentRequirements": requirements
        })
    }

    fn request_from(json: serde_json::Value) -> v2::VerifyRequest {
        v2::VerifyRequest::from_verify(r402_protocol::payment::VerifyRequest::from(json)).unwrap()
    }

    #[test]
    fn accepts_a_well_formed_payment() {
        let validated = validate_at(&request_from(request_json()), NOW).unwrap();
        assert_eq!(validated.payer.to_string(), PAYER);
        assert_eq!(validated.pay_to.to_string(), PAYEE);
        assert_eq!(validated.asset.to_string(), ASSET);
        assert_eq!(validated.chain, CasperChainReference::CASPER_TEST);
    }

    #[test]
    fn rejects_network_mismatch_between_payload_and_requirements() {
        let mut json = request_json();
        json["paymentPayload"]["accepted"]["network"] = serde_json::json!("casper:casper");
        let err = validate_at(&request_from(json), NOW).unwrap_err();
        assert!(matches!(err, CasperExactError::NetworkMismatch { .. }));
    }

    #[test]
    fn rejects_accepted_amount_mismatch() {
        let mut json = request_json();
        json["paymentPayload"]["accepted"]["amount"] = serde_json::json!("1");
        let err = validate_at(&request_from(json), NOW).unwrap_err();
        assert_eq!(err, CasperExactError::AcceptedRequirementsMismatch);
    }

    #[test]
    fn rejects_accepted_pay_to_mismatch() {
        let mut json = request_json();
        json["paymentPayload"]["accepted"]["payTo"] = serde_json::json!(PAYER);
        let err = validate_at(&request_from(json), NOW).unwrap_err();
        assert_eq!(err, CasperExactError::AcceptedRequirementsMismatch);
    }

    #[test]
    fn rejects_accepted_asset_mismatch() {
        let mut json = request_json();
        json["paymentPayload"]["accepted"]["asset"] = serde_json::json!("ab".repeat(32));
        let err = validate_at(&request_from(json), NOW).unwrap_err();
        assert_eq!(err, CasperExactError::AcceptedRequirementsMismatch);
    }

    #[test]
    fn rejects_accepted_max_timeout_mismatch() {
        let mut json = request_json();
        json["paymentPayload"]["accepted"]["maxTimeoutSeconds"] = serde_json::json!(999);
        let err = validate_at(&request_from(json), NOW).unwrap_err();
        assert_eq!(err, CasperExactError::AcceptedRequirementsMismatch);
    }

    #[test]
    fn rejects_accepted_extra_mismatch() {
        let mut json = request_json();
        json["paymentPayload"]["accepted"]["extra"] =
            serde_json::json!({ "name": "Other Token", "version": "1" });
        let err = validate_at(&request_from(json), NOW).unwrap_err();
        assert_eq!(err, CasperExactError::AcceptedRequirementsMismatch);
    }

    #[test]
    fn rejects_unknown_casper_network() {
        let mut json = request_json();
        json["paymentPayload"]["accepted"]["network"] = serde_json::json!("casper:casper-dev");
        json["paymentRequirements"]["network"] = serde_json::json!("casper:casper-dev");
        let err = validate_at(&request_from(json), NOW).unwrap_err();
        assert!(matches!(err, CasperExactError::UnsupportedNetwork(_)));
    }

    #[test]
    fn rejects_tagged_asset_hash() {
        let mut json = request_json();
        json["paymentRequirements"]["asset"] = serde_json::json!(format!("01{ASSET}"));
        json["paymentPayload"]["accepted"]["asset"] = serde_json::json!(format!("01{ASSET}"));
        let err = validate_at(&request_from(json), NOW).unwrap_err();
        assert!(matches!(err, CasperExactError::InvalidAsset(_)));
    }

    #[test]
    fn rejects_pay_to_mismatch() {
        let mut json = request_json();
        json["paymentPayload"]["payload"]["authorization"]["to"] = serde_json::json!(PAYER);
        let err = validate_at(&request_from(json), NOW).unwrap_err();
        assert!(matches!(err, CasperExactError::PayToMismatch { .. }));
    }

    #[test]
    fn rejects_amount_mismatch() {
        let mut json = request_json();
        json["paymentPayload"]["payload"]["authorization"]["value"] =
            serde_json::json!("1400000000");
        let err = validate_at(&request_from(json), NOW).unwrap_err();
        assert!(matches!(err, CasperExactError::AmountMismatch { .. }));
    }

    #[test]
    fn rejects_zero_amount() {
        let mut json = request_json();
        json["paymentRequirements"]["amount"] = serde_json::json!("0");
        json["paymentPayload"]["accepted"]["amount"] = serde_json::json!("0");
        json["paymentPayload"]["payload"]["authorization"]["value"] = serde_json::json!("0");
        let err = validate_at(&request_from(json), NOW).unwrap_err();
        assert_eq!(err, CasperExactError::ZeroAmount);
    }

    #[test]
    fn rejects_missing_eip712_domain_fields() {
        let mut json = request_json();
        json["paymentRequirements"]["extra"] = serde_json::json!({ "name": "", "version": "1" });
        json["paymentPayload"]["accepted"]["extra"] =
            serde_json::json!({ "name": "", "version": "1" });
        assert_eq!(
            validate_at(&request_from(json), NOW).unwrap_err(),
            CasperExactError::MissingTokenName
        );

        let mut json_version = request_json();
        json_version["paymentRequirements"]["extra"] =
            serde_json::json!({ "name": "Wrapped CSPR", "version": "  " });
        json_version["paymentPayload"]["accepted"]["extra"] =
            serde_json::json!({ "name": "Wrapped CSPR", "version": "  " });
        assert_eq!(
            validate_at(&request_from(json_version), NOW).unwrap_err(),
            CasperExactError::MissingTokenVersion
        );
    }

    #[test]
    fn rejects_contract_hash_as_payer() {
        let mut json = request_json();
        json["paymentPayload"]["payload"]["authorization"]["from"] =
            serde_json::json!(format!("01{ASSET}"));
        let err = validate_at(&request_from(json), NOW).unwrap_err();
        assert!(matches!(
            err,
            CasperExactError::InvalidPayer(_) | CasperExactError::PublicKeyMismatch
        ));
    }

    #[test]
    fn rejects_public_key_that_does_not_derive_from() {
        let mut json = request_json();
        json["paymentPayload"]["payload"]["publicKey"] =
            serde_json::json!(format!("01{}", "bb".repeat(32)));
        json["paymentPayload"]["payload"]["signature"] =
            serde_json::json!(format!("01{}", "aa".repeat(64)));
        let err = validate_at(&request_from(json), NOW).unwrap_err();
        assert_eq!(err, CasperExactError::PublicKeyMismatch);
    }

    #[test]
    fn rejects_signature_algorithm_tag_mismatch() {
        let mut json = request_json();
        json["paymentPayload"]["payload"]["signature"] =
            serde_json::json!(format!("01{}", "aa".repeat(64)));
        let err = validate_at(&request_from(json), NOW).unwrap_err();
        assert!(matches!(err, CasperExactError::InvalidSignature(_)));
    }

    #[test]
    fn timing_window_boundaries() {
        assert!(matches!(
            validate_timing(100, 500, 100, 300),
            Err(CasperExactError::NotYetValid { .. })
        ));
        assert!(validate_timing(100, 500, 101, 300).is_ok());
        assert!(matches!(
            validate_timing(100, 200, 200, 6),
            Err(CasperExactError::Expired { .. })
        ));
        assert!(matches!(
            validate_timing(100, 200, 150, 60),
            Err(CasperExactError::Expired { .. })
        ));
        assert!(validate_timing(100, 500, 200, 300).is_ok());
    }

    #[test]
    fn rejects_expired_authorization_end_to_end() {
        let err = validate_at(&request_from(request_json()), 1_700_000_599).unwrap_err();
        assert!(matches!(err, CasperExactError::Expired { .. }));
    }

    #[test]
    fn rejects_malformed_signature_before_semantic_checks() {
        let mut json = request_json();
        json["paymentPayload"]["payload"]["signature"] = serde_json::json!("aa".repeat(10));
        let err = validate_at(&request_from(json), NOW).unwrap_err();
        assert!(matches!(err, CasperExactError::InvalidSignature(_)));
    }
}
