//! Off-chain verification for EVM `auth-capture`.

use alloy_primitives::{Address, B256, U256};
use alloy_sol_types::{SolStruct, eip712_domain, sol};
use r402_core::error::VerificationError;
use r402_core::wire::UnixTimestamp;

use super::nonce::{compute_payer_agnostic_payment_info_hash, hash_as_uint256};
use super::types::{
    AUTH_CAPTURE_CLOCK_SKEW_SECS, AuthCaptureExtra, AuthCapturePayload,
    EIP3009_TOKEN_COLLECTOR_ADDRESS, PERMIT2_TOKEN_COLLECTOR_ADDRESS, PaymentInfo,
};
use crate::auth_capture::types::v2;
use crate::exact::PERMIT2_ADDRESS;

sol! {
    struct ReceiveWithAuthorization {
        address from;
        address to;
        uint256 value;
        uint256 validAfter;
        uint256 validBefore;
        bytes32 nonce;
    }

    struct TokenPermissions {
        address token;
        uint256 amount;
    }

    struct PermitTransferFrom {
        TokenPermissions permitted;
        address spender;
        uint256 nonce;
        uint256 deadline;
    }
}

/// Reconstructs on-chain `PaymentInfo` from requirements + payload salt/payer.
#[must_use]
pub const fn reconstruct_payment_info(
    requirements: &v2::PaymentRequirements,
    extra: &AuthCaptureExtra,
    payer: Address,
    salt: B256,
    pre_approval_expiry: u64,
) -> PaymentInfo {
    PaymentInfo {
        operator: extra.capture_authorizer.0,
        payer,
        receiver: requirements.pay_to.0,
        token: requirements.asset.0,
        max_amount: requirements.amount.0,
        pre_approval_expiry,
        authorization_expiry: extra.capture_deadline,
        refund_expiry: extra.refund_deadline,
        min_fee_bps: extra.min_fee_bps,
        max_fee_bps: extra.max_fee_bps,
        fee_receiver: extra.fee_recipient.0,
        salt: U256::from_be_bytes(salt.0),
    }
}

/// Runs off-chain verification steps 1–12 from the auth-capture EVM spec.
///
/// Returns the recovered payer address on success.
///
/// # Errors
///
/// Structured [`VerificationError`] for each failed check.
pub fn verify_offchain(
    payload: &v2::PaymentPayload,
    requirements: &v2::PaymentRequirements,
    chain_id: u64,
) -> Result<Address, VerificationError> {
    if requirements.scheme != r402_core::scheme::AuthCaptureScheme {
        return Err(VerificationError::UnsupportedScheme);
    }
    let accepted_scheme = payload.accepted.scheme.to_string();
    if accepted_scheme != "auth-capture" {
        return Err(VerificationError::AcceptedRequirementsMismatch);
    }
    if payload.accepted.network != requirements.network {
        return Err(VerificationError::ChainIdMismatch);
    }

    let extra = requirements
        .extra
        .as_ref()
        .ok_or_else(|| VerificationError::InvalidFormat("missing auth-capture extra".into()))?;

    validate_extra(extra)?;

    let method = extra.transfer_method();
    if payload.payload.transfer_method() != method {
        return Err(VerificationError::InvalidFormat(
            "payload assetTransferMethod mismatch".into(),
        ));
    }

    let now = UnixTimestamp::now().as_secs();
    if extra.refund_deadline < extra.capture_deadline {
        return Err(VerificationError::InvalidFormat(
            "refundDeadline must be >= captureDeadline".into(),
        ));
    }
    if extra.capture_deadline <= now.saturating_add(AUTH_CAPTURE_CLOCK_SKEW_SECS) {
        return Err(VerificationError::Expired);
    }

    match &payload.payload {
        AuthCapturePayload::Eip3009(p) => verify_eip3009(p, requirements, extra, chain_id, now),
        AuthCapturePayload::Permit2(p) => verify_permit2(p, requirements, extra, chain_id, now),
    }
}

fn validate_extra(extra: &AuthCaptureExtra) -> Result<(), VerificationError> {
    if extra.name.is_empty() || extra.version.is_empty() {
        return Err(VerificationError::InvalidFormat(
            "extra.name and extra.version required".into(),
        ));
    }
    if extra.max_fee_bps < extra.min_fee_bps {
        return Err(VerificationError::InvalidFormat(
            "maxFeeBps must be >= minFeeBps".into(),
        ));
    }
    Ok(())
}

fn verify_eip3009(
    p: &super::types::AuthCaptureEip3009Payload,
    requirements: &v2::PaymentRequirements,
    extra: &AuthCaptureExtra,
    chain_id: u64,
    now: u64,
) -> Result<Address, VerificationError> {
    let auth = &p.authorization;
    if auth.to != EIP3009_TOKEN_COLLECTOR_ADDRESS {
        return Err(VerificationError::InvalidFormat(
            "authorization.to must be EIP3009_TOKEN_COLLECTOR_ADDRESS".into(),
        ));
    }
    if auth.value.0 != requirements.amount.0 {
        return Err(VerificationError::InvalidPaymentAmount);
    }
    let valid_before = auth.valid_before.as_secs();
    if valid_before <= now.saturating_add(AUTH_CAPTURE_CLOCK_SKEW_SECS) {
        return Err(VerificationError::Expired);
    }
    if valid_before > extra.capture_deadline {
        return Err(VerificationError::InvalidFormat(
            "validBefore must be <= captureDeadline".into(),
        ));
    }
    if auth.valid_after.as_secs() > now {
        return Err(VerificationError::Early);
    }

    let info = reconstruct_payment_info(requirements, extra, auth.from, p.salt, valid_before);
    let expected_nonce = compute_payer_agnostic_payment_info_hash(chain_id, &info);
    if auth.nonce != expected_nonce {
        return Err(VerificationError::InvalidFormat(
            "authorization.nonce does not match PaymentInfo hash".into(),
        ));
    }

    let domain = eip712_domain! {
        name: extra.name.clone(),
        version: extra.version.clone(),
        chain_id: chain_id,
        verifying_contract: requirements.asset.0,
    };
    let typed = ReceiveWithAuthorization {
        from: auth.from,
        to: auth.to,
        value: auth.value.0,
        validAfter: U256::from(auth.valid_after.as_secs()),
        validBefore: U256::from(valid_before),
        nonce: auth.nonce,
    };
    let hash = typed.eip712_signing_hash(&domain);
    recover_payer(hash, &p.signature, auth.from)?;
    Ok(auth.from)
}

fn verify_permit2(
    p: &super::types::AuthCapturePermit2Payload,
    requirements: &v2::PaymentRequirements,
    extra: &AuthCaptureExtra,
    chain_id: u64,
    now: u64,
) -> Result<Address, VerificationError> {
    let a = &p.permit2_authorization;
    if a.spender != PERMIT2_TOKEN_COLLECTOR_ADDRESS {
        return Err(VerificationError::InvalidFormat(
            "spender must be PERMIT2_TOKEN_COLLECTOR_ADDRESS".into(),
        ));
    }
    if a.permitted.token != requirements.asset.0 {
        return Err(VerificationError::AssetMismatch);
    }
    if a.permitted.amount.0 != requirements.amount.0 {
        return Err(VerificationError::InvalidPaymentAmount);
    }
    let deadline: u64 = a.deadline.0.try_into().unwrap_or(u64::MAX);
    if deadline <= now.saturating_add(AUTH_CAPTURE_CLOCK_SKEW_SECS) {
        return Err(VerificationError::Expired);
    }
    if deadline > extra.capture_deadline {
        return Err(VerificationError::InvalidFormat(
            "deadline must be <= captureDeadline".into(),
        ));
    }

    let info = reconstruct_payment_info(requirements, extra, a.from, p.salt, deadline);
    let expected = compute_payer_agnostic_payment_info_hash(chain_id, &info);
    if a.nonce.0 != hash_as_uint256(expected) {
        return Err(VerificationError::InvalidFormat(
            "permit2 nonce does not match PaymentInfo hash".into(),
        ));
    }

    let domain = eip712_domain! {
        name: "Permit2",
        chain_id: chain_id,
        verifying_contract: PERMIT2_ADDRESS,
    };
    let typed = PermitTransferFrom {
        permitted: TokenPermissions {
            token: a.permitted.token,
            amount: a.permitted.amount.0,
        },
        spender: a.spender,
        nonce: a.nonce.0,
        deadline: a.deadline.0,
    };
    let hash = typed.eip712_signing_hash(&domain);
    recover_payer(hash, &p.signature, a.from)?;
    Ok(a.from)
}

fn recover_payer(
    hash: B256,
    signature: &alloy_primitives::Bytes,
    expected: Address,
) -> Result<(), VerificationError> {
    // Off-chain /verify is the access gate. This path only recovers 65-byte
    // EOA signatures; any other length is unchecked, so reject it.
    if signature.len() != 65 {
        return Err(VerificationError::InvalidSignature(
            "signature must be 65 bytes".into(),
        ));
    }
    let sig = alloy_primitives::Signature::from_raw(signature.as_ref())
        .map_err(|e| VerificationError::InvalidSignature(format!("parse signature: {e}")))?;
    let recovered = sig
        .recover_address_from_prehash(&hash)
        .map_err(|e| VerificationError::InvalidSignature(format!("recover: {e}")))?;
    if recovered != expected {
        return Err(VerificationError::InvalidSignature(
            "recovered signer mismatch".into(),
        ));
    }
    Ok(())
}
