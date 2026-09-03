//! Off-chain witness and extra checks for the EIP-155 upto scheme.

use alloy_primitives::{Address, U256};
use r402_protocol::error::VerificationError;
use r402_protocol::payment::UnixTimestamp;
#[cfg(feature = "telemetry")]
use tracing::instrument;

use crate::eip2612::Eip2612SignedPermit;
use crate::permit2::PERMIT2_ADDRESS;
use crate::signature::assert_time;
use crate::upto::{UptoPermit2Authorization, X402_UPTO_PERMIT2_PROXY, payload};

/// Shared verify/settle preconditions (scheme, `pay_to`, asset, spender, time, facilitator).
///
/// Amount is intentionally omitted: verify requires equality, settle requires `≤`.
#[cfg_attr(feature = "telemetry", instrument(skip_all, err))]
pub(super) fn assert_offchain_valid_shared(
    payload: &payload::v2::PaymentPayload,
    requirements: &payload::v2::PaymentRequirements,
    facilitator_signers: &[Address],
    clock_skew_tolerance: u64,
) -> Result<(), VerificationError> {
    let accepted = &payload.accepted;
    if accepted.scheme != requirements.scheme
        || accepted.network != requirements.network
        || accepted.asset != requirements.asset
        || accepted.pay_to != requirements.pay_to
    {
        return Err(VerificationError::AcceptedRequirementsMismatch);
    }

    let auth = &payload.payload.permit2_authorization;

    if auth.spender != X402_UPTO_PERMIT2_PROXY {
        return Err(VerificationError::InvalidSignature(
            "invalid Permit2 spender: must be x402UptoPermit2Proxy".into(),
        ));
    }
    if auth.witness.to != Address::from(accepted.pay_to) {
        return Err(VerificationError::RecipientMismatch);
    }
    if auth.permitted.token != Address::from(accepted.asset) {
        return Err(VerificationError::AssetMismatch);
    }

    let expected_facilitator = requirements
        .extra
        .as_ref()
        .map(|e| Address::from(e.facilitator_address));
    if expected_facilitator != Some(auth.witness.facilitator) {
        return Err(VerificationError::AcceptedRequirementsMismatch);
    }

    assert_facilitator_authorized(auth.witness.facilitator, facilitator_signers)?;

    if let Some(eip2612) = Eip2612SignedPermit::from_extensions(&payload.extensions)
        .map_err(|e| VerificationError::InvalidFormat(e.to_string()))?
    {
        assert_eip2612_consistent_with_permit2(&eip2612, auth)?;
    }

    let valid_after_secs: u64 = auth.witness.valid_after.0.try_into().unwrap_or(u64::MAX);
    let deadline_secs: u64 = auth.deadline.0.try_into().unwrap_or(u64::MAX);
    assert_time(
        UnixTimestamp::from_secs(valid_after_secs),
        UnixTimestamp::from_secs(deadline_secs),
        clock_skew_tolerance,
    )?;

    Ok(())
}

fn assert_eip2612_consistent_with_permit2(
    eip2612: &Eip2612SignedPermit,
    auth: &UptoPermit2Authorization,
) -> Result<(), VerificationError> {
    if eip2612.from != auth.from {
        return Err(VerificationError::InvalidFormat(
            "eip2612GasSponsoring.from does not match permit2Authorization.from".into(),
        ));
    }
    if eip2612.asset != auth.permitted.token {
        return Err(VerificationError::InvalidFormat(
            "eip2612GasSponsoring.asset does not match permit2Authorization.permitted.token".into(),
        ));
    }
    if eip2612.spender != PERMIT2_ADDRESS {
        return Err(VerificationError::InvalidFormat(
            "eip2612GasSponsoring.spender must be the canonical Permit2 contract".into(),
        ));
    }
    let permit2_value: U256 = auth.permitted.amount.into();
    let eip2612_value: U256 = eip2612.amount.into();
    if permit2_value != eip2612_value {
        return Err(VerificationError::InvalidFormat(format!(
            "eip2612GasSponsoring.value ({eip2612_value}) does not match permit2Authorization.permitted.amount ({permit2_value})"
        )));
    }
    Ok(())
}

pub(super) fn assert_facilitator_authorized(
    witness_facilitator: Address,
    signer_addresses: &[Address],
) -> Result<(), VerificationError> {
    if signer_addresses.contains(&witness_facilitator) {
        Ok(())
    } else {
        Err(VerificationError::UptoFacilitatorMismatch {
            witness: witness_facilitator.to_checksum(None),
        })
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256};
    use r402_protocol::error::VerificationError;

    use super::*;
    use crate::chain::ChecksummedAddress;
    use crate::upto::UptoPaymentRequirementsExtra;
    use crate::upto::facilitator::verify::fixtures::{TEST_FACILITATOR, base_payload, signers};

    #[test]
    fn shared_checks_reject_wrong_spender() {
        let (mut payload, reqs) = base_payload(U256::from(100u64));
        payload.payload.permit2_authorization.spender = Address::ZERO;
        let err = assert_offchain_valid_shared(&payload, &reqs, &signers(), 30).unwrap_err();
        assert!(matches!(err, VerificationError::InvalidSignature(_)));
    }

    #[test]
    fn shared_checks_reject_recipient_mismatch() {
        let (mut payload, reqs) = base_payload(U256::from(100u64));
        payload.payload.permit2_authorization.witness.to = Address::repeat_byte(0xEE);
        let err = assert_offchain_valid_shared(&payload, &reqs, &signers(), 30).unwrap_err();
        assert!(matches!(err, VerificationError::RecipientMismatch));
    }

    #[test]
    fn shared_checks_reject_asset_mismatch() {
        let (mut payload, reqs) = base_payload(U256::from(100u64));
        payload.payload.permit2_authorization.permitted.token = Address::repeat_byte(0xEE);
        let err = assert_offchain_valid_shared(&payload, &reqs, &signers(), 30).unwrap_err();
        assert!(matches!(err, VerificationError::AssetMismatch));
    }

    #[test]
    fn shared_checks_reject_requirements_asset_mismatch() {
        let (payload, mut reqs) = base_payload(U256::from(100u64));
        reqs.asset = ChecksummedAddress::from(Address::repeat_byte(0x01));
        let err = assert_offchain_valid_shared(&payload, &reqs, &signers(), 30).unwrap_err();
        assert!(matches!(
            err,
            VerificationError::AcceptedRequirementsMismatch
        ));
    }

    #[test]
    fn shared_checks_reject_witness_facilitator_mismatch_against_extra() {
        let (mut payload, reqs) = base_payload(U256::from(100u64));
        payload.payload.permit2_authorization.witness.facilitator = Address::repeat_byte(0xEE);
        let err = assert_offchain_valid_shared(&payload, &reqs, &signers(), 30).unwrap_err();
        assert!(matches!(
            err,
            VerificationError::AcceptedRequirementsMismatch
        ));
    }

    #[test]
    fn shared_checks_reject_facilitator_not_in_signer_set() {
        let other_facilitator = Address::repeat_byte(0xDD);
        let (mut payload, mut reqs) = base_payload(U256::from(100u64));
        payload.payload.permit2_authorization.witness.facilitator = other_facilitator;
        payload.accepted.extra = Some(UptoPaymentRequirementsExtra::new(ChecksummedAddress::from(
            other_facilitator,
        )));
        reqs.extra = Some(UptoPaymentRequirementsExtra::new(ChecksummedAddress::from(
            other_facilitator,
        )));
        let err = assert_offchain_valid_shared(&payload, &reqs, &signers(), 30).unwrap_err();
        match err {
            VerificationError::UptoFacilitatorMismatch { witness } => {
                assert_eq!(witness, other_facilitator.to_checksum(None));
            }
            other => unreachable!("expected UptoFacilitatorMismatch, got {other:?}"),
        }
    }

    #[test]
    fn shared_checks_reject_missing_facilitator_extra() {
        let (payload, mut reqs) = base_payload(U256::from(100u64));
        reqs.extra = None;
        let err = assert_offchain_valid_shared(&payload, &reqs, &signers(), 30).unwrap_err();
        assert!(matches!(
            err,
            VerificationError::AcceptedRequirementsMismatch
        ));
    }

    #[test]
    fn authorised_facilitator_is_accepted() {
        assert!(assert_facilitator_authorized(TEST_FACILITATOR, &signers()).is_ok());
    }

    mod eip2612_extension {
        use alloy_primitives::{Address, Bytes, U256};
        use r402_protocol::payment::{ExtensionEntry, UnixTimestamp};

        use super::*;
        use crate::chain::TokenAmount;
        use crate::eip2612::{EIP2612_GAS_SPONSORING_KEY, Eip2612SignedPermit};
        use crate::permit2::PERMIT2_ADDRESS;
        use crate::upto::facilitator::verify::fixtures::{base_payload, signers};

        fn sample_eip2612(value: U256) -> Eip2612SignedPermit {
            Eip2612SignedPermit {
                from: Address::repeat_byte(0xCC),
                asset: Address::repeat_byte(0xAA),
                spender: PERMIT2_ADDRESS,
                amount: TokenAmount::from(value),
                nonce: TokenAmount::from(U256::ZERO),
                deadline: TokenAmount::from(U256::from(UnixTimestamp::now().as_secs() + 300)),
                signature: Bytes::from(vec![0x42u8; 65]),
                version: crate::eip2612::EIP2612_GAS_SPONSORING_VERSION.into(),
            }
        }

        fn payload_with_eip2612(
            permitted: U256,
            mutate: impl FnOnce(&mut Eip2612SignedPermit),
        ) -> (
            payload::v2::PaymentPayload,
            payload::v2::PaymentRequirements,
        ) {
            let (mut payload, reqs) = base_payload(permitted);
            let mut eip2612 = sample_eip2612(permitted);
            mutate(&mut eip2612);
            payload.extensions.insert(
                EIP2612_GAS_SPONSORING_KEY,
                eip2612.to_extension_entry().expect("serialises"),
            );
            (payload, reqs)
        }

        #[test]
        fn shared_check_accepts_consistent_extension() {
            let (payload, reqs) = payload_with_eip2612(U256::from(5_000_000_u64), |_| {});
            assert_offchain_valid_shared(&payload, &reqs, &signers(), 30)
                .expect("consistent eip2612 sponsorship is accepted");
        }

        #[test]
        fn shared_check_rejects_from_mismatch() {
            let (payload, reqs) = payload_with_eip2612(U256::from(5_000_000_u64), |e| {
                e.from = Address::repeat_byte(0xEE);
            });
            let err = assert_offchain_valid_shared(&payload, &reqs, &signers(), 30).unwrap_err();
            assert!(matches!(err, VerificationError::InvalidFormat(msg) if msg.contains("from")));
        }

        #[test]
        fn shared_check_rejects_asset_mismatch() {
            let (payload, reqs) = payload_with_eip2612(U256::from(5_000_000_u64), |e| {
                e.asset = Address::repeat_byte(0xEE);
            });
            let err = assert_offchain_valid_shared(&payload, &reqs, &signers(), 30).unwrap_err();
            assert!(matches!(err, VerificationError::InvalidFormat(msg) if msg.contains("asset")));
        }

        #[test]
        fn shared_check_rejects_non_canonical_spender() {
            let (payload, reqs) = payload_with_eip2612(U256::from(5_000_000_u64), |e| {
                e.spender = Address::repeat_byte(0xEE);
            });
            let err = assert_offchain_valid_shared(&payload, &reqs, &signers(), 30).unwrap_err();
            assert!(
                matches!(err, VerificationError::InvalidFormat(msg) if msg.contains("Permit2"))
            );
        }

        #[test]
        fn shared_check_rejects_value_mismatch() {
            let (payload, reqs) = payload_with_eip2612(U256::from(5_000_000_u64), |e| {
                e.amount = TokenAmount::from(U256::from(1_000_000_u64));
            });
            let err = assert_offchain_valid_shared(&payload, &reqs, &signers(), 30).unwrap_err();
            assert!(matches!(err, VerificationError::InvalidFormat(msg) if msg.contains("value")));
        }

        #[test]
        fn shared_check_rejects_malformed_extension_envelope() {
            let (mut payload, reqs) = base_payload(U256::from(5_000_000_u64));
            payload.extensions.insert(
                EIP2612_GAS_SPONSORING_KEY,
                ExtensionEntry::raw(serde_json::json!({"from": "not-an-address"})),
            );
            let err = assert_offchain_valid_shared(&payload, &reqs, &signers(), 30).unwrap_err();
            assert!(matches!(err, VerificationError::InvalidFormat(_)));
        }
    }
}
