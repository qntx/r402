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

use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::Provider;
use alloy_sol_types::{Eip712Domain, SolInterface, SolStruct, eip712_domain};
use r402_core::chain::ChainId;
use r402_core::error::VerificationError;
use r402_core::wire::UnixTimestamp;
#[cfg(feature = "telemetry")]
use tracing::instrument;

use super::contract::IX402UptoPermit2Proxy;
use super::contract::IX402UptoPermit2Proxy::IX402UptoPermit2ProxyErrors;
use crate::chain::Eip155ChainReference;
use crate::error::Eip155ExactError;
use crate::exact::PERMIT2_ADDRESS;
use crate::exact::VALIDATOR_ADDRESS;
use crate::exact::facilitator::assert_time;
use crate::exact::facilitator::contract::{IERC20, Validator6492};
use crate::exact::facilitator::signature::StructuredSignature;
use crate::upto::types::{
    self, PermitWitnessTransferFrom as SolPermitWitnessTransferFrom,
    TokenPermissions as SolTokenPermissions, Witness as SolWitness,
};
use crate::upto::{Eip2612SignedPermit, UptoPermit2Payload, X402_UPTO_PERMIT2_PROXY};

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
    /// Optional buyer-signed EIP-2612 permit. When `Some`, both verify and
    /// settle route through `proxy.settleWithPermit`, atomically setting
    /// the canonical Permit2 allowance before transferring funds. The
    /// off-chain consistency invariants (`from`, `asset`, `spender`,
    /// `value`) are enforced by [`assert_offchain_valid_shared`] before
    /// this struct is constructed.
    pub eip2612: Option<Eip2612SignedPermit>,
}

impl PreparedUptoPermit2 {
    /// Builds the prepared data from a raw payload, reconstructing the
    /// EIP-712 hash, parsing the structured signature, and extracting
    /// the optional EIP-2612 gas-sponsoring extension.
    pub(super) fn try_new(
        chain_reference: Eip155ChainReference,
        payload: &UptoPermit2Payload,
        extensions: &r402_core::wire::Extensions,
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

/// Cross-validates the EIP-2612 sponsorship permit against the embedded
/// Permit2 authorization. The proxy contract enforces these invariants
/// at settle time (`Permit2612AmountMismatch`, `InvalidPermit2Address`,
/// etc.); we re-check them off-chain to fail fast and produce richer
/// wire-level diagnostics.
fn assert_eip2612_consistent_with_permit2(
    eip2612: &Eip2612SignedPermit,
    auth: &crate::upto::UptoPermit2Authorization,
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

/// Maps a revert payload returned by the upto Permit2 proxy onto the
/// matching [`VerificationError`] variant.
///
/// The proxy contract emits one of seven custom errors (see
/// [`IX402UptoPermit2Proxy`] for the full list). Three of them have a
/// dedicated wire-level reason in x402 v2:
///
/// | Solidity error             | `VerificationError` variant            |
/// | -------------------------- | --------------------------------------- |
/// | `AmountExceedsPermitted`   | [`UptoAmountExceedsPermitted`]          |
/// | `UnauthorizedFacilitator`  | [`UptoUnauthorizedFacilitator`]         |
/// | `PaymentTooEarly`          | [`Early`]                               |
///
/// The remaining four errors (`Permit2612AmountMismatch`,
/// `ReentrancyGuardReentrantCall`, `InvalidDestination`, `InvalidOwner`,
/// `InvalidPermit2Address`) are operator-side invariants that should not
/// surface during normal operation; we return [`None`] for them so the
/// caller can fall back to a generic [`SimulationFailed`] with the raw
/// transport message.
///
/// Returns [`None`] when the bytes are empty, encode an unknown selector,
/// or fail strict ABI decoding.
///
/// [`UptoAmountExceedsPermitted`]: VerificationError::UptoAmountExceedsPermitted
/// [`UptoUnauthorizedFacilitator`]: VerificationError::UptoUnauthorizedFacilitator
/// [`Early`]: VerificationError::Early
/// [`SimulationFailed`]: VerificationError::SimulationFailed
fn classify_upto_revert_bytes(data: &[u8]) -> Option<VerificationError> {
    let decoded = IX402UptoPermit2ProxyErrors::abi_decode(data).ok()?;
    Some(match decoded {
        IX402UptoPermit2ProxyErrors::AmountExceedsPermitted(_) => {
            VerificationError::UptoAmountExceedsPermitted
        }
        IX402UptoPermit2ProxyErrors::UnauthorizedFacilitator(_) => {
            VerificationError::UptoUnauthorizedFacilitator
        }
        IX402UptoPermit2ProxyErrors::PaymentTooEarly(_) => VerificationError::Early,
        IX402UptoPermit2ProxyErrors::Permit2612AmountMismatch(_)
        | IX402UptoPermit2ProxyErrors::ReentrancyGuardReentrantCall(_)
        | IX402UptoPermit2ProxyErrors::InvalidDestination(_)
        | IX402UptoPermit2ProxyErrors::InvalidOwner(_)
        | IX402UptoPermit2ProxyErrors::InvalidPermit2Address(_) => return None,
    })
}

/// Lowers a wire [`Eip2612SignedPermit`] into the ABI struct consumed
/// by `proxy.settleWithPermit`.
///
/// The signature MUST be exactly 65 bytes (`r ‖ s ‖ v`); this is
/// enforced by [`Eip2612SignedPermit::from_extensions`] when the wire
/// payload is parsed, so failure here implies the `prepared` value
/// originated from a non-extension code path.
pub(super) fn build_eip2612_abi(
    eip2612: &Eip2612SignedPermit,
) -> Result<IX402UptoPermit2Proxy::EIP2612Permit, VerificationError> {
    let (r, s, v) = eip2612.split_signature().ok_or_else(|| {
        VerificationError::InvalidFormat(
            "eip2612GasSponsoring signature is not the canonical 65-byte (r,s,v) form".into(),
        )
    })?;
    Ok(IX402UptoPermit2Proxy::EIP2612Permit {
        value: eip2612.amount.into(),
        deadline: eip2612.deadline.into(),
        r,
        s,
        v,
    })
}

/// Returns [`VerificationError::InvalidFormat`] when an EIP-2612
/// sponsorship is paired with a non-EOA Permit2 signature.
///
/// Rationale: a valid EIP-2612 permit requires an ECDSA signature that
/// the token contract can verify via `ecrecover`, which in turn
/// requires the buyer to control an EOA private key. Smart-contract
/// wallets expose ERC-1271 / ERC-6492 instead, and cannot produce
/// EIP-2612 permits themselves.
pub(super) fn assert_eip2612_supported_signature_kind(
    prepared: &PreparedUptoPermit2,
) -> Result<(), VerificationError> {
    if prepared.eip2612.is_some()
        && !matches!(prepared.structured_signature, StructuredSignature::Eoa(_))
    {
        return Err(VerificationError::InvalidFormat(
            "eip2612GasSponsoring requires an EOA Permit2 signature".into(),
        ));
    }
    Ok(())
}

/// Translates an [`alloy_contract::Error`] coming out of `proxy.settle.call()`
/// into a precise [`VerificationError`].
///
/// First attempts to decode the revert data via
/// [`classify_upto_revert_bytes`]; on miss, falls back to wrapping the
/// transport-layer message in [`VerificationError::SimulationFailed`] so
/// the diagnostic is still preserved end-to-end.
#[allow(
    clippy::needless_pass_by_value,
    reason = "intended to be used as a `map_err` closure target which transfers ownership"
)]
fn classify_settle_call_error(err: alloy_contract::Error) -> VerificationError {
    if let Some(data) = err.as_revert_data()
        && let Some(reason) = classify_upto_revert_bytes(&data)
    {
        return reason;
    }
    VerificationError::SimulationFailed(err.to_string())
}

/// Same as [`classify_settle_call_error`] but for the multicall path used
/// in the EIP-6492 batched simulation.
///
/// `aggregate3` returns the revert payload as
/// [`alloy_provider::Failure::return_data`]; we feed it through the same
/// classifier so the wire-level reason is identical regardless of
/// signature kind.
#[allow(
    clippy::needless_pass_by_value,
    reason = "intended to be used as a `map_err` closure target which transfers ownership"
)]
fn classify_settle_multicall_failure(failure: alloy_provider::Failure) -> VerificationError {
    classify_upto_revert_bytes(&failure.return_data)
        .unwrap_or_else(|| VerificationError::SimulationFailed(failure.to_string()))
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
    assert_eip2612_supported_signature_kind(prepared)?;

    let erc20 = IERC20::new(prepared.token, provider);

    // Run allowance + balance checks in parallel.
    //
    // The Permit2 allowance check is suppressed when the buyer attached
    // an EIP-2612 sponsorship: the proxy will atomically grant the
    // allowance via `token.permit(...)` before invoking Permit2, so a
    // pre-existing allowance is not required.
    let allowance_call = erc20.allowance(prepared.from, PERMIT2_ADDRESS);
    let balance_call = erc20.balanceOf(prepared.from);
    let (allowance_result, balance_result) =
        tokio::join!(allowance_call.call(), balance_call.call());

    if prepared.eip2612.is_none()
        && let Ok(allowance) = allowance_result
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
            settle_result.map_err(classify_settle_multicall_failure)?;
        }
        StructuredSignature::Eoa(signature) => {
            let sig_bytes: Bytes = signature.as_bytes().into();
            if let Some(eip2612) = &prepared.eip2612 {
                let eip2612_abi = build_eip2612_abi(eip2612)?;
                let call = proxy.settleWithPermit(
                    eip2612_abi,
                    permit,
                    prepared.max_amount,
                    prepared.from,
                    witness,
                    sig_bytes,
                );
                let simulation = call.call().into_future();
                traced!(
                    simulation,
                    tracing::info_span!(
                        "simulate_upto_permit2_settle_with_permit",
                        from = %prepared.from,
                        to = %prepared.to,
                        token = %prepared.token,
                        max_amount = %prepared.max_amount,
                        sig_kind = "EOA",
                        sponsoring = "eip2612",
                        otel.kind = "client",
                    )
                )
                .map_err(classify_settle_call_error)?;
            } else {
                let call = proxy.settle(
                    permit,
                    prepared.max_amount,
                    prepared.from,
                    witness,
                    sig_bytes,
                );
                let simulation = call.call().into_future();
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
                .map_err(classify_settle_call_error)?;
            }
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
            .map_err(classify_settle_call_error)?;
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
    let prepared = PreparedUptoPermit2::try_new(chain, &payload.payload, &payload.extensions)?;
    assert_onchain_valid(provider, &prepared, max_amount).await
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, Bytes, U256};
    use r402_core::chain::ChainId;

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

        let requirements = types::v2::PaymentRequirements::new(
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
        let pay_payload = types::v2::PaymentPayload::new(requirements.clone(), payload_body);
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

    /// Tests covering the EIP-2612 `eip2612GasSponsoring` extension:
    /// off-chain consistency invariants and prepared-payload routing.
    mod eip2612_extension {
        use r402_core::wire;

        use super::super::*;
        use super::*;
        use crate::upto::{EIP2612_GAS_SPONSORING_KEY, Eip2612SignedPermit};

        fn sample_eip2612(value: U256) -> Eip2612SignedPermit {
            Eip2612SignedPermit {
                from: Address::repeat_byte(0xCC),
                asset: Address::repeat_byte(0xAA),
                spender: PERMIT2_ADDRESS,
                amount: TokenAmount::from(value),
                nonce: TokenAmount::from(U256::ZERO),
                deadline: TokenAmount::from(U256::from(UnixTimestamp::now().as_secs() + 300)),
                signature: Bytes::from(vec![0x42u8; 65]),
                version: crate::EIP2612_GAS_SPONSORING_VERSION.into(),
            }
        }

        fn payload_with_eip2612(
            permitted: U256,
            mutate: impl FnOnce(&mut Eip2612SignedPermit),
        ) -> (types::v2::PaymentPayload, types::v2::PaymentRequirements) {
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
                wire::ExtensionEntry::raw(serde_json::json!({"from": "not-an-address"})),
            );
            let err = assert_offchain_valid_shared(&payload, &reqs, &signers(), 30).unwrap_err();
            assert!(matches!(err, VerificationError::InvalidFormat(_)));
        }

        #[test]
        fn build_eip2612_abi_splits_signature_correctly() {
            let mut sig = Vec::with_capacity(65);
            sig.extend_from_slice(&[0x11u8; 32]);
            sig.extend_from_slice(&[0x22u8; 32]);
            sig.push(27);
            let permit = Eip2612SignedPermit {
                signature: Bytes::from(sig),
                ..sample_eip2612(U256::from(1u64))
            };
            let abi = build_eip2612_abi(&permit).expect("splits 65-byte sig");
            assert_eq!(abi.r.as_slice(), [0x11u8; 32]);
            assert_eq!(abi.s.as_slice(), [0x22u8; 32]);
            assert_eq!(abi.v, 27);
            assert_eq!(abi.value, U256::from(1u64));
        }
    }

    /// Round-trips every concrete proxy revert through
    /// [`classify_upto_revert_bytes`] using its own ABI encoding. Drift
    /// between the encoder and decoder (e.g. selector renames) would
    /// surface here before it reaches users.
    mod revert_classification {
        use alloy_sol_types::SolError;

        use super::super::*;
        use crate::upto::facilitator::contract::IX402UptoPermit2Proxy::{
            AmountExceedsPermitted, InvalidDestination, InvalidOwner, InvalidPermit2Address,
            PaymentTooEarly, Permit2612AmountMismatch, ReentrancyGuardReentrantCall,
            UnauthorizedFacilitator,
        };

        #[test]
        fn amount_exceeds_permitted_maps_to_dedicated_reason() {
            let bytes = AmountExceedsPermitted {}.abi_encode();
            assert!(matches!(
                classify_upto_revert_bytes(&bytes),
                Some(VerificationError::UptoAmountExceedsPermitted)
            ));
        }

        #[test]
        fn unauthorized_facilitator_maps_to_dedicated_reason() {
            let bytes = UnauthorizedFacilitator {}.abi_encode();
            assert!(matches!(
                classify_upto_revert_bytes(&bytes),
                Some(VerificationError::UptoUnauthorizedFacilitator)
            ));
        }

        #[test]
        fn payment_too_early_maps_to_early() {
            let bytes = PaymentTooEarly {}.abi_encode();
            assert!(matches!(
                classify_upto_revert_bytes(&bytes),
                Some(VerificationError::Early)
            ));
        }

        #[test]
        fn operator_invariants_fall_through_to_simulation_failed() {
            // Each of these is an operator-side bug rather than a buyer
            // mistake, so they intentionally don't map to a wire reason
            // and the caller falls back to SimulationFailed with the raw
            // transport message.
            for bytes in [
                Permit2612AmountMismatch {}.abi_encode(),
                ReentrancyGuardReentrantCall {}.abi_encode(),
                InvalidDestination {}.abi_encode(),
                InvalidOwner {}.abi_encode(),
                InvalidPermit2Address {}.abi_encode(),
            ] {
                assert!(
                    classify_upto_revert_bytes(&bytes).is_none(),
                    "expected no wire-level mapping for raw bytes {bytes:?}"
                );
            }
        }

        #[test]
        fn unknown_selector_returns_none() {
            // 4 bytes of arbitrary non-selector data -> decoder returns
            // Err -> classifier returns None.
            assert!(classify_upto_revert_bytes(&[0xDE, 0xAD, 0xBE, 0xEF]).is_none());
        }

        #[test]
        fn empty_bytes_return_none() {
            assert!(classify_upto_revert_bytes(&[]).is_none());
        }
    }
}
