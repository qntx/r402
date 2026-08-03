//! Payload validation for the Casper "exact" payment scheme.
//!
//! These checks are the chain-agnostic half of Casper verification: shape,
//! address and asset well-formedness, amount equality, and the timing
//! window. They run identically on the client (before a payment is sent),
//! on the resource server (before a payment is relayed), and inside the
//! facilitator client (before a request leaves the process), so a malformed
//! payment is rejected without a network round trip.
//!
//! Cryptographic verification of the EIP-712 signature and the on-chain
//! `transfer_with_authorization` call are performed by the settlement
//! facilitator, which holds the signing key and the node connection.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::chain::{Address, CasperChainReference, ContractPackageHash};
use crate::exact::types::{MIN_SETTLEMENT_WINDOW_SECS, v2};
use crate::exact::{CasperExactError, ExactCasperPayload};

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
/// Returns the first [`CasperExactError`] encountered. Checks are ordered so
/// that structural failures surface before semantic ones, which keeps error
/// messages actionable.
pub fn validate_request(request: &v2::VerifyRequest) -> Result<ValidatedPayment, CasperExactError> {
    validate_at(request, now_unix())
}

/// Validates a request against an explicit wall-clock time.
///
/// Exposed so tests and replay tooling can pin `now` deterministically.
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

    if accepted.network != requirements.network {
        return Err(CasperExactError::NetworkMismatch {
            payload: accepted.network.to_string(),
            requirements: requirements.network.to_string(),
        });
    }

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

    validate_timing(authorization.valid_after, authorization.valid_before, now)?;

    Ok(ValidatedPayment {
        payer: authorization.from,
        pay_to,
        asset,
        chain,
    })
}

/// Validates the structural invariants of a payload: fixed-width signature,
/// fixed-width nonce, and an account-hash payer.
///
/// # Errors
///
/// Returns [`CasperExactError`] when a field is malformed.
pub fn validate_payload_shape(payload: &ExactCasperPayload) -> Result<(), CasperExactError> {
    let _ = payload.signature_bytes()?;
    let _ = payload.authorization.nonce_bytes()?;

    if !payload.authorization.from.is_account() {
        return Err(CasperExactError::InvalidPayer(
            payload.authorization.from.to_string(),
        ));
    }
    if !payload.authorization.to.is_account() {
        return Err(CasperExactError::InvalidPayTo(
            payload.authorization.to.to_string(),
        ));
    }
    Ok(())
}

/// Checks the authorisation validity window against `now`.
///
/// A payment must already be valid, must not have expired, and must leave at
/// least [`MIN_SETTLEMENT_WINDOW_SECS`] for the settlement transaction to be
/// included in a block.
///
/// # Errors
///
/// Returns [`CasperExactError::NotYetValid`] or [`CasperExactError::Expired`].
pub const fn validate_timing(
    valid_after: u64,
    valid_before: u64,
    now: u64,
) -> Result<(), CasperExactError> {
    if valid_after > now {
        return Err(CasperExactError::NotYetValid { valid_after, now });
    }
    if valid_before <= now {
        return Err(CasperExactError::Expired { valid_before, now });
    }
    if valid_before - now < MIN_SETTLEMENT_WINDOW_SECS {
        return Err(CasperExactError::Expired { valid_before, now });
    }
    Ok(())
}

/// Returns the current Unix timestamp in seconds.
///
/// Clocks before the Unix epoch are clamped to `0`; the timing checks then
/// fail closed rather than accepting an expired authorisation.
#[must_use]
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYER: &str = "001234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
    const PAYEE: &str = "00fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321";
    const ASSET: &str = "3d80df21ba4ee4d66a2a1f60c32570dd5685e4b279f6538162a5fd1314847c1e";
    const NOW: u64 = 1_700_000_100;

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
                    "signature": "aa".repeat(65),
                    "publicKey": format!("01{}", "bb".repeat(32)),
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
        v2::VerifyRequest::from_verify(r402_core::wire::VerifyRequest::from(json)).unwrap()
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
    fn rejects_unknown_casper_network() {
        let mut json = request_json();
        json["paymentPayload"]["accepted"]["network"] = serde_json::json!("casper:casper-dev");
        json["paymentRequirements"]["network"] = serde_json::json!("casper:casper-dev");
        let err = validate_at(&request_from(json), NOW).unwrap_err();
        assert!(matches!(err, CasperExactError::UnsupportedNetwork(_)));
    }

    #[test]
    fn rejects_tagged_asset_hash() {
        // `asset` is an untagged package hash; a 66-char tagged key is wrong.
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
        assert_eq!(
            validate_at(&request_from(json), NOW).unwrap_err(),
            CasperExactError::MissingTokenName
        );

        let mut json = request_json();
        json["paymentRequirements"]["extra"] =
            serde_json::json!({ "name": "Wrapped CSPR", "version": "  " });
        assert_eq!(
            validate_at(&request_from(json), NOW).unwrap_err(),
            CasperExactError::MissingTokenVersion
        );
    }

    #[test]
    fn rejects_contract_hash_as_payer() {
        let mut json = request_json();
        json["paymentPayload"]["payload"]["authorization"]["from"] =
            serde_json::json!(format!("01{ASSET}"));
        let err = validate_at(&request_from(json), NOW).unwrap_err();
        assert!(matches!(err, CasperExactError::InvalidPayer(_)));
    }

    #[test]
    fn timing_window_boundaries() {
        // Not yet valid.
        assert!(matches!(
            validate_timing(100, 200, 99),
            Err(CasperExactError::NotYetValid { .. })
        ));
        // Exactly at validAfter is acceptable.
        assert!(validate_timing(100, 200, 100).is_ok());
        // Already expired.
        assert!(matches!(
            validate_timing(100, 200, 200),
            Err(CasperExactError::Expired { .. })
        ));
        // Too close to expiry to realistically settle.
        assert!(matches!(
            validate_timing(100, 200, 195),
            Err(CasperExactError::Expired { .. })
        ));
        // Exactly the minimum settlement window is acceptable.
        assert!(validate_timing(100, 200, 200 - MIN_SETTLEMENT_WINDOW_SECS).is_ok());
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

    #[test]
    fn now_unix_is_after_the_2020s_epoch() {
        assert!(
            now_unix() > 1_600_000_000,
            "system clock should be past 2020"
        );
    }
}
