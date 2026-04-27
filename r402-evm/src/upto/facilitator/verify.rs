//! Verification logic for the EIP-155 upto scheme.
//!
//! Phase-dependent `amount` semantics (x402 v2 upto spec §5):
//!
//! - verify phase — `paymentRequirements.amount` equals the authorised
//!   maximum (the value the buyer signed over)
//! - settle phase — `paymentRequirements.amount` equals the actual amount
//!   to settle and MUST be `≤` the authorised maximum
//!
//! [`assert_offchain_valid_verify`] enforces the verify-phase invariant;
//! [`assert_offchain_valid_settle`] enforces the settle-phase one.

use std::future::IntoFuture;

use alloy_primitives::{Address, U256};
use alloy_provider::Provider;
use alloy_sol_types::{Eip712Domain, SolStruct, eip712_domain};
use r402_core::chain::ChainId;
use r402_core::error::VerificationError;
use r402_core::wire::UnixTimestamp;
#[cfg(feature = "telemetry")]
use tracing::instrument;

use super::contract::IX402UptoPermit2Proxy;
use crate::chain::Eip155ChainReference;
use crate::exact::PERMIT2_ADDRESS;
use crate::exact::facilitator::contract::{IERC20, Validator6492};
use crate::exact::facilitator::signature::StructuredSignature;
use crate::exact::facilitator::{Eip155ExactError, VALIDATOR_ADDRESS, assert_time};
use crate::upto::types::{
    self, PermitWitnessTransferFrom as SolPermitWitnessTransferFrom,
    TokenPermissions as SolTokenPermissions, Witness as SolWitness,
};
use crate::upto::{UptoPermit2Payload, X402_UPTO_PERMIT2_PROXY};

/// Macro copied from the exact facilitator to keep telemetry spans
/// consistent across both schemes.
macro_rules! traced {
    ($fut:expr, $span:expr) => {{
        #[cfg(feature = "telemetry")]
        {
            use tracing::Instrument;
            $fut.instrument($span).await
        }
        #[cfg(not(feature = "telemetry"))]
        {
            $fut.await
        }
    }};
}

/// Fully prepared data from a Permit2 upto payload.
///
/// Intermediate bundle carried between verify and settle to avoid reparsing.
#[derive(Debug)]
pub(super) struct PreparedUptoPermit2 {
    /// Sender / owner address.
    pub from: Address,
    /// Recipient address.
    pub to: Address,
    /// Facilitator address authorised in the witness (`witness.facilitator`).
    /// The on-chain proxy enforces `msg.sender == facilitator`, so this MUST
    /// equal the address that ultimately submits the settle transaction.
    pub facilitator: Address,
    /// Token contract.
    pub token: Address,
    /// Authorised maximum amount (equals `payload.permitted.amount`).
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
}

impl PreparedUptoPermit2 {
    /// Builds the prepared data from a raw payload, reconstructing the
    /// EIP-712 hash and parsing the structured signature.
    pub(super) fn try_new(
        chain_reference: Eip155ChainReference,
        payload: &UptoPermit2Payload,
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
        let structured_signature = StructuredSignature::try_from_bytes(
            payload.signature.clone(),
            auth.from,
            &eip712_hash,
        )?;

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
        })
    }
}

/// Returns the Permit2 EIP-712 domain (`name = "Permit2"`, no version,
/// `verifyingContract = PERMIT2_ADDRESS`).
pub(super) const fn upto_permit2_domain(chain: Eip155ChainReference) -> Eip712Domain {
    eip712_domain! {
        name: "Permit2",
        chain_id: chain.inner(),
        verifying_contract: PERMIT2_ADDRESS,
    }
}

/// Runs all off-chain preconditions shared by verify and settle.
///
/// Checks scheme / network / `pay_to` / asset / spender / token / time bounds,
/// and that:
/// 1. the signed permit amount equals the declared authorised maximum,
/// 2. `witness.facilitator` matches `requirements.extra.facilitatorAddress`
///    (the address the resource server announced),
/// 3. `witness.facilitator` is one of the facilitator's known signers.
#[cfg_attr(feature = "telemetry", instrument(skip_all, err))]
pub(super) fn assert_offchain_valid_shared(
    payload: &types::v2::PaymentPayload,
    requirements: &types::v2::PaymentRequirements,
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

    // Spec §2: the buyer signs `witness.facilitator` against the address
    // the server published in `extra.facilitatorAddress`. Any divergence
    // is treated as `AcceptedRequirementsMismatch` because the buyer is
    // signing for a different counterparty than the server expects.
    let expected_facilitator = requirements
        .extra
        .as_ref()
        .map(|e| Address::from(e.facilitator_address));
    if expected_facilitator != Some(auth.witness.facilitator) {
        return Err(VerificationError::AcceptedRequirementsMismatch);
    }

    // Spec §2 (TS / Go reference): the witness facilitator MUST be one of
    // the facilitator's announced signer addresses. Otherwise we wouldn't
    // be able to satisfy `msg.sender == witness.facilitator` on settle.
    super::assert_facilitator_authorized(auth.witness.facilitator, facilitator_signers)?;

    let valid_after_secs: u64 = auth.witness.valid_after.0.try_into().unwrap_or(u64::MAX);
    let deadline_secs: u64 = auth.deadline.0.try_into().unwrap_or(u64::MAX);
    assert_time(
        UnixTimestamp::from_secs(valid_after_secs),
        UnixTimestamp::from_secs(deadline_secs),
        clock_skew_tolerance,
    )?;

    Ok(())
}

/// Verify-phase check: the signed maximum MUST equal
/// `paymentRequirements.amount`, which at verify-time carries the declared
/// authorised maximum (x402 v2 upto spec §5).
pub(super) fn assert_offchain_valid_verify(
    payload: &types::v2::PaymentPayload,
    requirements: &types::v2::PaymentRequirements,
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

/// Settle-phase check: `paymentRequirements.amount` is the actual settlement
/// amount; it MUST be `≤` the signed maximum (x402 v2 upto spec §5, §4.1).
pub(super) fn assert_offchain_valid_settle(
    payload: &types::v2::PaymentPayload,
    requirements: &types::v2::PaymentRequirements,
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

/// On-chain preconditions: sufficient ERC-20 allowance on Permit2, sufficient
/// balance, and a successful `eth_call` simulation of `proxy.settle(...)`
/// with the worst-case (maximum) amount.
///
/// Running the simulation inside verify catches revert conditions (expired
/// nonce, revoked allowance, short deadline) before gas is committed, in
/// line with Fix-7 for the exact scheme.
#[cfg_attr(feature = "telemetry", instrument(skip_all, err))]
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_lines,
    reason = "signature-dispatch branches are inherently nested and span EIP-6492 / EOA / EIP-1271 arms"
)]
pub(super) async fn assert_onchain_valid<P: Provider>(
    provider: &P,
    prepared: &PreparedUptoPermit2,
    required_amount: U256,
) -> Result<Address, Eip155ExactError> {
    let erc20 = IERC20::new(prepared.token, provider);

    // Run allowance + balance checks in parallel.
    let allowance_call = erc20.allowance(prepared.from, PERMIT2_ADDRESS);
    let balance_call = erc20.balanceOf(prepared.from);
    let (allowance_result, balance_result) =
        tokio::join!(allowance_call.call(), balance_call.call());

    if let Ok(allowance) = allowance_result
        && allowance < required_amount
    {
        return Err(VerificationError::Permit2AllowanceRequired.into());
    }
    if let Ok(balance) = balance_result
        && balance < required_amount
    {
        return Err(VerificationError::InsufficientFunds.into());
    }

    let proxy = IX402UptoPermit2Proxy::new(X402_UPTO_PERMIT2_PROXY, provider);
    let permit = IX402UptoPermit2Proxy::Permit {
        permitted: IX402UptoPermit2Proxy::TokenPermissions {
            token: prepared.token,
            amount: prepared.max_amount,
        },
        nonce: prepared.nonce,
        deadline: prepared.deadline,
    };
    let witness = IX402UptoPermit2Proxy::Witness {
        to: prepared.to,
        facilitator: prepared.facilitator,
        validAfter: prepared.valid_after,
    };

    match &prepared.structured_signature {
        StructuredSignature::EIP6492 {
            factory: _,
            factory_calldata: _,
            inner,
            original,
        } => {
            let validator6492 = Validator6492::new(VALIDATOR_ADDRESS, provider);
            let is_valid_call = validator6492.isValidSigWithSideEffects(
                prepared.from,
                prepared.eip712_hash,
                original.clone(),
            );
            // Simulate with max amount (worst-case).
            let settle_call = proxy.settle(
                permit,
                prepared.max_amount,
                prepared.from,
                witness,
                inner.clone(),
            );
            let aggregate = provider.multicall().add(is_valid_call).add(settle_call);
            let (is_valid, settle_result) = traced!(
                aggregate.aggregate3(),
                tracing::info_span!(
                    "simulate_upto_permit2_settle",
                    from = %prepared.from,
                    to = %prepared.to,
                    token = %prepared.token,
                    max_amount = %prepared.max_amount,
                    sig_kind = "EIP6492",
                    otel.kind = "client",
                )
            )?;
            let is_valid =
                is_valid.map_err(|e| VerificationError::InvalidSignature(e.to_string()))?;
            if !is_valid {
                return Err(VerificationError::InvalidSignature(
                    "chain reported signature to be invalid".into(),
                )
                .into());
            }
            settle_result.map_err(|e| VerificationError::SimulationFailed(e.to_string()))?;
        }
        StructuredSignature::Eoa(signature) => {
            let settle_call = proxy.settle(
                permit,
                prepared.max_amount,
                prepared.from,
                witness,
                signature.as_bytes().into(),
            );
            let simulation = settle_call.call().into_future();
            traced!(
                simulation,
                tracing::info_span!(
                    "simulate_upto_permit2_settle",
                    from = %prepared.from,
                    to = %prepared.to,
                    token = %prepared.token,
                    max_amount = %prepared.max_amount,
                    sig_kind = "EOA",
                    otel.kind = "client",
                )
            )
            .map_err(|e| VerificationError::SimulationFailed(e.to_string()))?;
        }
        StructuredSignature::EIP1271(signature) => {
            let settle_call = proxy.settle(
                permit,
                prepared.max_amount,
                prepared.from,
                witness,
                signature.clone(),
            );
            let simulation = settle_call.call().into_future();
            traced!(
                simulation,
                tracing::info_span!(
                    "simulate_upto_permit2_settle",
                    from = %prepared.from,
                    to = %prepared.to,
                    token = %prepared.token,
                    max_amount = %prepared.max_amount,
                    sig_kind = "EIP1271",
                    otel.kind = "client",
                )
            )
            .map_err(|e| VerificationError::SimulationFailed(e.to_string()))?;
        }
    }

    Ok(prepared.from)
}

/// Composite verify entry: runs the off-chain amount / spender / witness
/// checks, then the on-chain allowance / balance / simulation battery.
pub(super) async fn verify_permit2_upto_payment<P: Provider>(
    provider: &P,
    chain: Eip155ChainReference,
    payload: &types::v2::PaymentPayload,
    requirements: &types::v2::PaymentRequirements,
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
    let prepared = PreparedUptoPermit2::try_new(chain, &payload.payload)?;
    assert_onchain_valid(provider, &prepared, max_amount).await
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, Bytes, U256};
    use r402_core::chain::ChainId;
    use r402_core::wire;

    use super::*;
    use crate::chain::{ChecksummedAddress, TokenAmount};
    use crate::exact::Permit2TokenPermissions;
    use crate::upto::{
        UptoPaymentRequirementsExtra, UptoPermit2Authorization, UptoPermit2Witness, UptoScheme,
    };

    /// Stable facilitator signer used across tests; mirrors what an
    /// `Eip155UptoFacilitator::supported()` response would publish.
    const TEST_FACILITATOR: Address = Address::repeat_byte(0xFA);

    fn base_payload(
        permitted_amount: U256,
    ) -> (types::v2::PaymentPayload, types::v2::PaymentRequirements) {
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

        let requirements = types::v2::PaymentRequirements {
            scheme: UptoScheme,
            network,
            amount: TokenAmount::from(permitted_amount),
            pay_to,
            asset,
            max_timeout_seconds: 300,
            extra: Some(UptoPaymentRequirementsExtra::new(ChecksummedAddress::from(
                facilitator_addr,
            ))),
        };
        let pay_payload = types::v2::PaymentPayload {
            x402_version: wire::V2,
            accepted: requirements.clone(),
            resource: None,
            payload: payload_body,
            extensions: wire::Extensions::new(),
        };
        (pay_payload, requirements)
    }

    fn signers() -> [Address; 1] {
        [TEST_FACILITATOR]
    }

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
        // Buyer signed for an unauthorised facilitator address.
        payload.payload.permit2_authorization.witness.facilitator = Address::repeat_byte(0xEE);
        let err = assert_offchain_valid_shared(&payload, &reqs, &signers(), 30).unwrap_err();
        assert!(matches!(
            err,
            VerificationError::AcceptedRequirementsMismatch
        ));
    }

    #[test]
    fn shared_checks_reject_facilitator_not_in_signer_set() {
        // Witness and accepted.extra agree on facilitator X, but the
        // facilitator's signer set doesn't include X.
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
}
