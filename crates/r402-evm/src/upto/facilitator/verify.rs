//! Verify/settle orchestration for the EIP-155 upto scheme.
//!
//! Amount is not compared in the shared witness checks: verify requires
//! equality with the signed max; settle requires `actual ≤ max`.

use alloy_primitives::{Address, U256};
use alloy_provider::Provider;
use alloy_sol_types::{Eip712Domain, SolStruct, eip712_domain};
use r402_protocol::error::VerificationError;
use r402_protocol::network::ChainId;
use r402_protocol::payment::Extensions;

use super::verify_permit2::assert_onchain_valid;
use super::verify_witness::assert_offchain_valid_shared;
use crate::chain::Eip155ChainReference;
use crate::eip2612::Eip2612SignedPermit;
use crate::error::Eip155ExactError;
use crate::permit2::PERMIT2_ADDRESS;
use crate::signature::StructuredSignature;
use crate::upto::UptoPermit2Payload;
use crate::upto::payload::{
    self, PermitWitnessTransferFrom as SolPermitWitnessTransferFrom,
    TokenPermissions as SolTokenPermissions, Witness as SolWitness,
};

/// Fully prepared data from a Permit2 upto payload.
#[derive(Debug)]
pub(super) struct PreparedUptoPermit2 {
    /// Sender / owner address.
    pub from: Address,
    /// Recipient address.
    pub to: Address,
    /// Facilitator address authorised in the witness.
    pub facilitator: Address,
    /// Token contract.
    pub token: Address,
    /// Authorised maximum amount (`payload.permitted.amount`).
    pub max_amount: U256,
    /// Permit2 nonce.
    pub nonce: U256,
    /// Deadline (unix seconds as `U256`).
    pub deadline: U256,
    /// `validAfter` (unix seconds as `U256`).
    pub valid_after: U256,
    /// EIP-712 signing hash for signature verification.
    pub eip712_hash: alloy_primitives::B256,
    /// Parsed signature structure (EOA / EIP-1271 / EIP-6492).
    pub structured_signature: StructuredSignature,
    /// Optional buyer-signed EIP-2612 permit.
    pub eip2612: Option<Eip2612SignedPermit>,
}

impl PreparedUptoPermit2 {
    pub(super) fn try_new(
        chain_reference: Eip155ChainReference,
        payload: &UptoPermit2Payload,
        extensions: &Extensions,
    ) -> Result<Self, Eip155ExactError> {
        let auth = &payload.permit2_authorization;
        let domain = upto_permit2_domain(chain_reference);

        let permit_witness = SolPermitWitnessTransferFrom {
            permitted: SolTokenPermissions {
                token: auth.permitted.token,
                amount: auth.permitted.amount.into(),
            },
            spender: auth.spender,
            nonce: auth.nonce.into(),
            deadline: auth.deadline.into(),
            witness: SolWitness {
                to: auth.witness.to,
                facilitator: auth.witness.facilitator,
                validAfter: auth.witness.valid_after.into(),
            },
        };
        let eip712_hash = permit_witness.eip712_signing_hash(&domain);
        let structured_signature = StructuredSignature::try_from_bytes(payload.signature.clone())?;
        let eip2612 = Eip2612SignedPermit::from_extensions(extensions)
            .map_err(|e| VerificationError::InvalidFormat(e.to_string()))?;

        Ok(Self {
            from: auth.from,
            to: auth.witness.to,
            facilitator: auth.witness.facilitator,
            token: auth.permitted.token,
            max_amount: auth.permitted.amount.into(),
            nonce: auth.nonce.into(),
            deadline: auth.deadline.into(),
            valid_after: auth.witness.valid_after.into(),
            eip712_hash,
            structured_signature,
            eip2612,
        })
    }
}

pub(super) const fn upto_permit2_domain(chain: Eip155ChainReference) -> Eip712Domain {
    eip712_domain! {
        name: "Permit2",
        chain_id: chain.inner(),
        verifying_contract: PERMIT2_ADDRESS,
    }
}

/// Verify-phase: signed maximum MUST equal `paymentRequirements.amount`.
pub(super) fn assert_offchain_valid_verify(
    payload: &payload::v2::PaymentPayload,
    requirements: &payload::v2::PaymentRequirements,
    facilitator_signers: &[Address],
    clock_skew_tolerance: u64,
) -> Result<U256, VerificationError> {
    assert_offchain_valid_shared(
        payload,
        requirements,
        facilitator_signers,
        clock_skew_tolerance,
    )?;
    let permitted: U256 = payload
        .payload
        .permit2_authorization
        .permitted
        .amount
        .into();
    let declared_max: U256 = requirements.amount.into();
    if permitted != declared_max {
        return Err(VerificationError::InvalidPaymentAmount);
    }
    Ok(declared_max)
}

/// Settle-phase: `paymentRequirements.amount` is the actual charge and MUST be ≤ signed max.
///
/// HTTP Sequential writes this actual via `UptoActualAmount` before settle.
pub(super) fn assert_offchain_valid_settle(
    payload: &payload::v2::PaymentPayload,
    requirements: &payload::v2::PaymentRequirements,
    facilitator_signers: &[Address],
    clock_skew_tolerance: u64,
) -> Result<U256, VerificationError> {
    assert_offchain_valid_shared(
        payload,
        requirements,
        facilitator_signers,
        clock_skew_tolerance,
    )?;
    let permitted: U256 = payload
        .payload
        .permit2_authorization
        .permitted
        .amount
        .into();
    let requested: U256 = requirements.amount.into();
    if requested > permitted {
        return Err(VerificationError::SettlementAmountExceedsPermitted {
            requested: requested.to_string(),
            authorised: permitted.to_string(),
        });
    }
    Ok(requested)
}

/// Composite verify entry: off-chain amount / witness checks, then on-chain simulation.
pub(super) async fn verify_permit2_upto_payment<P: Provider>(
    provider: &P,
    chain: Eip155ChainReference,
    payload: &payload::v2::PaymentPayload,
    requirements: &payload::v2::PaymentRequirements,
    facilitator_signers: &[Address],
    clock_skew_tolerance: u64,
) -> Result<Address, Eip155ExactError> {
    let chain_id: ChainId = chain.into();
    if payload.accepted.network != chain_id {
        return Err(VerificationError::ChainIdMismatch.into());
    }
    let max_amount = assert_offchain_valid_verify(
        payload,
        requirements,
        facilitator_signers,
        clock_skew_tolerance,
    )?;
    let prepared = PreparedUptoPermit2::try_new(chain, &payload.payload, &payload.extensions)?;
    assert_onchain_valid(provider, &prepared, max_amount).await
}

#[cfg(test)]
pub(super) mod fixtures {
    use alloy_primitives::{Address, Bytes, U256};
    use r402_protocol::network::ChainId;
    use r402_protocol::payment::UnixTimestamp;

    use super::*;
    use crate::chain::{ChecksummedAddress, TokenAmount};
    use crate::permit2::Permit2TokenPermissions;
    use crate::upto::{
        UptoPaymentRequirementsExtra, UptoPermit2Authorization, UptoPermit2Payload,
        UptoPermit2Witness, UptoScheme, X402_UPTO_PERMIT2_PROXY,
    };

    pub(crate) const TEST_FACILITATOR: Address = Address::repeat_byte(0xFA);

    pub(crate) fn base_payload(
        permitted_amount: U256,
    ) -> (
        payload::v2::PaymentPayload,
        payload::v2::PaymentRequirements,
    ) {
        let network = ChainId::new("eip155", "84532");
        let pay_to_addr = Address::repeat_byte(0xBB);
        let asset_addr = Address::repeat_byte(0xAA);
        let pay_to = ChecksummedAddress::from(pay_to_addr);
        let asset = ChecksummedAddress::from(asset_addr);
        let facilitator_addr = TEST_FACILITATOR;

        let now = UnixTimestamp::now().as_secs();
        let witness = UptoPermit2Witness {
            to: pay_to_addr,
            facilitator: facilitator_addr,
            valid_after: TokenAmount::from(U256::from(now.saturating_sub(60))),
        };
        let auth = UptoPermit2Authorization {
            from: Address::repeat_byte(0xCC),
            permitted: Permit2TokenPermissions {
                token: asset_addr,
                amount: TokenAmount::from(permitted_amount),
            },
            spender: X402_UPTO_PERMIT2_PROXY,
            nonce: TokenAmount::from(U256::from(1u64)),
            deadline: TokenAmount::from(U256::from(now + 300)),
            witness,
        };
        let payload_body = UptoPermit2Payload {
            signature: Bytes::from(vec![0u8; 65]),
            permit2_authorization: auth,
        };

        let requirements = payload::v2::PaymentRequirements::new(
            UptoScheme,
            network,
            TokenAmount::from(permitted_amount),
            pay_to,
            asset,
            300,
        )
        .with_extra(UptoPaymentRequirementsExtra::new(ChecksummedAddress::from(
            facilitator_addr,
        )));
        let pay_payload = payload::v2::PaymentPayload::new(requirements.clone(), payload_body);
        (pay_payload, requirements)
    }

    pub(crate) fn signers() -> [Address; 1] {
        [TEST_FACILITATOR]
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::U256;

    use super::fixtures::{base_payload, signers};
    use super::*;
    use crate::chain::TokenAmount;

    #[test]
    fn verify_phase_rejects_mismatched_permitted_amount() {
        let permitted = U256::from(5_000_000_u64);
        let (payload, mut reqs) = base_payload(permitted);
        reqs.amount = TokenAmount::from(U256::from(4_000_000_u64));
        let err = assert_offchain_valid_verify(&payload, &reqs, &signers(), 30).unwrap_err();
        assert!(matches!(err, VerificationError::InvalidPaymentAmount));
    }

    #[test]
    fn settle_phase_rejects_over_authorised_amount() {
        let permitted = U256::from(5_000_000_u64);
        let (payload, mut reqs) = base_payload(permitted);
        reqs.amount = TokenAmount::from(U256::from(6_000_000_u64));
        let err = assert_offchain_valid_settle(&payload, &reqs, &signers(), 30).unwrap_err();
        match err {
            VerificationError::SettlementAmountExceedsPermitted {
                requested,
                authorised,
            } => {
                assert_eq!(requested, "6000000");
                assert_eq!(authorised, "5000000");
            }
            other => unreachable!("expected SettlementAmountExceedsPermitted, got {other:?}"),
        }
    }

    #[test]
    fn settle_phase_accepts_any_amount_up_to_max() {
        let permitted = U256::from(5_000_000_u64);
        let (payload, mut reqs) = base_payload(permitted);
        for settle_amount in [0u64, 1u64, 2_500_000u64, 5_000_000u64] {
            reqs.amount = TokenAmount::from(U256::from(settle_amount));
            let actual = assert_offchain_valid_settle(&payload, &reqs, &signers(), 30)
                .expect("settle amount within max must be valid");
            assert_eq!(actual, U256::from(settle_amount));
        }
    }
}
