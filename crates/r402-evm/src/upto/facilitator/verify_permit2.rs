//! On-chain Permit2 proxy simulation for the EIP-155 upto scheme.

use std::future::IntoFuture;

use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::Provider;
use alloy_sol_types::SolInterface;
use r402_protocol::error::VerificationError;
use r402_protocol::payment::Extensions;
#[cfg(feature = "telemetry")]
use tracing::instrument;

use super::contract::IX402UptoPermit2Proxy;
use super::contract::IX402UptoPermit2Proxy::IX402UptoPermit2ProxyErrors;
use super::verify::PreparedUptoPermit2;
use crate::asset::VALIDATOR_ADDRESS;
use crate::chain::contracts::{IERC20, Validator6492};
use crate::eip2612::Eip2612SignedPermit;
use crate::erc20_approval::{
    assert_facilitator_can_fund, funding_deficit, rpc_validate_covering_approval,
    simulate_permit2_settle_with_erc20_approval,
};
use crate::error::Eip155ExactError;
use crate::exact::facilitator::{permit2_allowance_gate, permit2_extension_covers_allowance};
use crate::permit2::PERMIT2_ADDRESS;
use crate::signature::{ClassifiedSignature, classify_with_code};
use crate::upto::X402_UPTO_PERMIT2_PROXY;

/// Maps a proxy revert payload onto a [`VerificationError`].
///
/// Operator-side invariants (`Permit2612AmountMismatch`, reentrancy, destination,
/// owner, Permit2 address) return [`None`] so the caller falls back to
/// [`VerificationError::SimulationFailed`].
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

#[allow(
    clippy::needless_pass_by_value,
    reason = "map_err closure target transfers ownership"
)]
fn classify_settle_call_error(err: alloy_contract::Error) -> VerificationError {
    if let Some(data) = err.as_revert_data()
        && let Some(reason) = classify_upto_revert_bytes(&data)
    {
        return reason;
    }
    VerificationError::SimulationFailed(err.to_string())
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "map_err closure target transfers ownership"
)]
fn classify_settle_multicall_failure(failure: alloy_provider::Failure) -> VerificationError {
    classify_upto_revert_bytes(&failure.return_data)
        .unwrap_or_else(|| VerificationError::SimulationFailed(failure.to_string()))
}

/// Lowers a wire [`Eip2612SignedPermit`] into the ABI struct for `settleWithPermit`.
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

/// EIP-2612 permits require an ECDSA signature the token can `ecrecover`.
pub(super) fn assert_eip2612_supported_signature_kind(
    classified: &ClassifiedSignature,
    has_eip2612: bool,
) -> Result<(), VerificationError> {
    if has_eip2612 && !matches!(classified, ClassifiedSignature::Eoa(_)) {
        return Err(VerificationError::InvalidFormat(
            "eip2612GasSponsoring requires an EOA Permit2 signature".into(),
        ));
    }
    Ok(())
}

const fn permit_and_witness(
    prepared: &PreparedUptoPermit2,
) -> (
    IX402UptoPermit2Proxy::Permit,
    IX402UptoPermit2Proxy::Witness,
) {
    (
        IX402UptoPermit2Proxy::Permit {
            permitted: IX402UptoPermit2Proxy::TokenPermissions {
                token: prepared.token,
                amount: prepared.max_amount,
            },
            nonce: prepared.nonce,
            deadline: prepared.deadline,
        },
        IX402UptoPermit2Proxy::Witness {
            to: prepared.to,
            facilitator: prepared.facilitator,
            validAfter: prepared.valid_after,
        },
    )
}

/// On-chain preconditions: Permit2 allowance, balance, and `eth_call` of `proxy.settle`.
///
/// Allowance RPC failure and insufficient allowance are fail-closed (412) unless a
/// valid `eip2612GasSponsoring` or registered `erc20ApprovalGasSponsoring` covers them.
#[cfg_attr(feature = "telemetry", instrument(skip_all, err))]
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_lines,
    reason = "signature-dispatch branches for EIP-6492 / EOA / EIP-1271"
)]
pub(super) async fn assert_onchain_valid<P: Provider>(
    provider: &P,
    prepared: &PreparedUptoPermit2,
    required_amount: U256,
    extensions: &Extensions,
    clock_skew_tolerance: u64,
    chain_id: u64,
) -> Result<Address, Eip155ExactError> {
    let classified = classify_with_code(
        provider,
        prepared.from,
        prepared.structured_signature.clone(),
        &prepared.eip712_hash,
    )
    .await?;
    assert_eip2612_supported_signature_kind(&classified, prepared.eip2612.is_some())?;

    let erc20 = IERC20::new(prepared.token, provider);
    let allowance_call = erc20.allowance(prepared.from, PERMIT2_ADDRESS);
    let balance_call = erc20.balanceOf(prepared.from);
    let (allowance_result, balance_result) =
        tokio::join!(allowance_call.call(), balance_call.call());

    let allowance_rpc_failed = allowance_result.is_err();
    let allowance_for_gate = allowance_result.as_ref().copied().map_err(|_| ());
    match permit2_allowance_gate(allowance_for_gate, required_amount, false, false) {
        Ok(()) => {}
        Err(VerificationError::Permit2AllowanceRequired) => {
            if !permit2_extension_covers_allowance(
                extensions,
                prepared.from,
                prepared.token,
                chain_id,
                clock_skew_tolerance,
            )? {
                if allowance_rpc_failed {
                    #[cfg(feature = "telemetry")]
                    tracing::warn!("permit2_allowance_rpc_failed");
                }
                return Err(VerificationError::Permit2AllowanceRequired.into());
            }
        }
        Err(e) => return Err(e.into()),
    }
    if let Ok(balance) = balance_result
        && balance < required_amount
    {
        return Err(VerificationError::InsufficientFunds.into());
    }

    let covering_needed = matches!(
        permit2_allowance_gate(allowance_for_gate, required_amount, false, false),
        Err(VerificationError::Permit2AllowanceRequired)
    );
    let mut erc20_approval = prepared.erc20_approval.clone();
    if covering_needed
        && prepared.eip2612.is_none()
        && let Some(approval) = erc20_approval.as_ref()
    {
        erc20_approval = Some(
            rpc_validate_covering_approval(
                provider,
                approval,
                prepared.from,
                prepared.token,
                chain_id,
            )
            .await?,
        );
    }

    let proxy = IX402UptoPermit2Proxy::new(X402_UPTO_PERMIT2_PROXY, provider);
    let (permit, witness) = permit_and_witness(prepared);

    match &classified {
        ClassifiedSignature::EIP6492 {
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
        ClassifiedSignature::Eoa(signature) => {
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
            } else if let Some(approval) = erc20_approval.as_ref() {
                let native_balance = provider.get_balance(prepared.from).await?;
                let fund_value = funding_deficit(approval.native_cost, native_balance)?;
                assert_facilitator_can_fund(provider, prepared.facilitator, fund_value).await?;
                let settle_calldata = proxy
                    .settle(
                        permit,
                        prepared.max_amount,
                        prepared.from,
                        witness,
                        sig_bytes,
                    )
                    .calldata()
                    .clone();
                simulate_permit2_settle_with_erc20_approval(
                    provider,
                    approval,
                    prepared.from,
                    prepared.facilitator,
                    prepared.facilitator,
                    prepared.token,
                    fund_value,
                    X402_UPTO_PERMIT2_PROXY,
                    settle_calldata,
                )
                .await?;
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
        ClassifiedSignature::EIP1271(signature) => {
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

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, Bytes, U256};

    use super::*;
    use crate::chain::TokenAmount;
    use crate::eip2612::EIP2612_GAS_SPONSORING_VERSION;

    fn sample_eip2612(value: U256) -> Eip2612SignedPermit {
        Eip2612SignedPermit {
            from: Address::repeat_byte(0xCC),
            asset: Address::repeat_byte(0xAA),
            spender: PERMIT2_ADDRESS,
            amount: TokenAmount::from(value),
            nonce: TokenAmount::from(U256::ZERO),
            deadline: TokenAmount::from(U256::from(1_700_000_000_u64)),
            signature: Bytes::from(vec![0x42u8; 65]),
            version: EIP2612_GAS_SPONSORING_VERSION.into(),
        }
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
            assert!(classify_upto_revert_bytes(&[0xDE, 0xAD, 0xBE, 0xEF]).is_none());
        }

        #[test]
        fn empty_bytes_return_none() {
            assert!(classify_upto_revert_bytes(&[]).is_none());
        }
    }
}
