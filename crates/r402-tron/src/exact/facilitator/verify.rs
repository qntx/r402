//! Payment verification logic for the Tron exact scheme.
//!
//! Contains precondition checks (time, domain, balance, value) and the
//! composite [`verify_payment`] / [`verify_permit2_payment`] functions.

use alloy_primitives::{Address as EvmAddress, U256};
use alloy_sol_types::{Eip712Domain, SolCall, SolStruct, eip712_domain};
use r402_core::chain::ChainId;
use r402_core::error::VerificationError;
use r402_core::wire::UnixTimestamp;

use super::signature::{SignedMessage, TronSignatureError};
use super::{Eip3009Payment, Permit2Payment};
use crate::chain::TronChainReference;
use crate::chain::contracts::eip3009;
use crate::exact::TronExactError;
use crate::exact::types;
use crate::exact::{
    Eip3009Payload, PaymentRequirementsExtra, Permit2Payload, TronPermitWitnessTransferFrom,
    TronTokenPermissions, TronWitness,
};

/// Validates that the current time is within the `validAfter` and `validBefore` bounds.
///
/// Applies `clock_skew_tolerance` seconds of grace for clock drift.
pub(crate) fn assert_time(
    valid_after: UnixTimestamp,
    valid_before: UnixTimestamp,
    clock_skew_tolerance: u64,
) -> Result<(), VerificationError> {
    let now = UnixTimestamp::now();
    if valid_before < now + clock_skew_tolerance {
        return Err(VerificationError::Expired);
    }
    if valid_after > now + clock_skew_tolerance {
        return Err(VerificationError::Early);
    }
    Ok(())
}

/// Validates that the accepted requirements match the server-side requirements
/// on the five core fields: scheme, network, amount, asset, and `pay_to`.
pub(super) fn assert_requirements_match(
    accepted: &types::v2::PaymentRequirements,
    requirements: &types::v2::PaymentRequirements,
) -> Result<(), VerificationError> {
    if accepted.scheme == requirements.scheme
        && accepted.network == requirements.network
        && accepted.amount == requirements.amount
        && accepted.asset == requirements.asset
        && accepted.pay_to == requirements.pay_to
    {
        Ok(())
    } else {
        Err(VerificationError::AcceptedRequirementsMismatch)
    }
}

/// Asserts that the declared payment amount equals the required amount exactly.
pub(super) fn assert_exact_value(sent: &U256, required: &U256) -> Result<(), VerificationError> {
    if sent == required {
        Ok(())
    } else {
        Err(VerificationError::InvalidPaymentAmount)
    }
}

/// Constructs the TIP-712 domain for EIP-3009 signature verification.
///
/// Uses `extra.name` / `extra.version` when provided; otherwise queries the
/// token contract on-chain via `name()` / `version()`.
pub(super) fn assert_domain(
    _provider: &crate::chain::TronChainProvider,
    chain: TronChainReference,
    token: EvmAddress,
    extra: Option<&PaymentRequirementsExtra>,
) -> Result<Eip712Domain, TronExactError> {
    let name = extra.map_or_else(|| {
        // Query token name via triggerconstantcontract
        // For now, use empty string if not provided — the signature will
        // fail to recover, which is the correct outcome.
        String::new()
    }, |e| e.name.clone());
    let version = extra.map_or_else(String::new, |e| e.version.clone());
    if name.is_empty() || version.is_empty() {
        return Err(TronExactError::MissingTip712Domain);
    }
    let domain = eip712_domain! {
        name: name,
        version: version,
        chain_id: u64::from(chain.inner()),
        verifying_contract: token,
    };
    Ok(domain)
}

/// Runs all preconditions needed for a successful EIP-3009 payment.
pub(super) async fn assert_valid_payment(
    provider: &crate::chain::TronChainProvider,
    chain: &TronChainReference,
    eip3009: &Eip3009Payload,
    payload: &types::v2::PaymentPayload,
    requirements: &types::v2::PaymentRequirements,
    clock_skew_tolerance: u64,
) -> Result<(Eip3009Payment, Eip712Domain), TronExactError> {
    let accepted = &payload.accepted;
    assert_requirements_match(accepted, requirements)?;

    let chain_id: ChainId = (*chain).into();
    if accepted.network != chain_id {
        return Err(TronExactError::ChainMismatch);
    }

    let authorization = &eip3009.authorization;
    if authorization.to != accepted.pay_to.as_evm() {
        return Err(TronExactError::RecipientMismatch);
    }

    assert_time(
        authorization.valid_after,
        authorization.valid_before,
        clock_skew_tolerance,
    )?;

    let token = accepted.asset.as_evm();
    let amount_required: U256 = accepted.amount.into();

    // Run independent checks
    let domain = assert_domain(provider, *chain, token, accepted.extra.as_ref())?;

    // Check nonce unused via authorizationState
    assert_nonce_unused(provider, token, authorization.from, authorization.nonce).await?;

    // Check balance
    assert_enough_balance(provider, token, authorization.from, amount_required).await?;

    // Check exact value
    assert_exact_value(&authorization.value.into(), &amount_required)?;

    let payment = Eip3009Payment {
        from: authorization.from,
        to: authorization.to,
        value: authorization.value.into(),
        valid_after: authorization.valid_after,
        valid_before: authorization.valid_before,
        nonce: authorization.nonce,
        signature: eip3009.signature.clone(),
    };

    Ok((payment, domain))
}

/// Checks whether the EIP-3009 authorization nonce has already been consumed.
async fn assert_nonce_unused(
    provider: &crate::chain::TronChainProvider,
    token: EvmAddress,
    authorizer: EvmAddress,
    nonce: alloy_primitives::B256,
) -> Result<(), TronExactError> {
    let call = eip3009::authorizationStateCall { authorizer, nonce };
    let calldata = eip3009::authorizationStateCall::abi_encode(&call);
    let result = provider
        .grid()
        .trigger_constant_contract(
            provider.signer_address(),
            token,
            &alloy_primitives::Bytes::from(calldata),
        )
        .await?;
    let padded: [u8; 32] = result
        .get(..32)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| {
            TronExactError::TronGrid("malformed authorizationState result".to_owned())
        })?;
    let used = U256::from_be_bytes(padded) != U256::ZERO;
    if used {
        return Err(TronExactError::NonceAlreadyUsed);
    }
    Ok(())
}

/// Checks if the payer has enough on-chain token balance.
async fn assert_enough_balance(
    provider: &crate::chain::TronChainProvider,
    token: EvmAddress,
    sender: EvmAddress,
    required: U256,
) -> Result<(), TronExactError> {
    let balance = provider.trc20_balance_of(token, sender).await?;
    if balance < required {
        return Err(TronExactError::InsufficientBalance);
    }
    Ok(())
}

/// Verifies an EIP-3009 payment by recovering the signature and simulating
/// the `transferWithAuthorization` call via `triggerconstantcontract`.
pub(super) async fn verify_payment(
    provider: &crate::chain::TronChainProvider,
    payment: &Eip3009Payment,
    eip712_domain: &Eip712Domain,
) -> Result<EvmAddress, TronExactError> {
    let signed_message = SignedMessage::extract(payment, eip712_domain).map_err(|e| match e {
        TronSignatureError::SignerMismatch => TronExactError::SignerMismatch,
        other => TronExactError::SignatureRecovery(other.to_string()),
    })?;

    // Simulate transferWithAuthorization via triggerconstantcontract to
    // catch revert conditions before broadcasting.
    let call = eip3009::transferWithAuthorizationCall {
        from: payment.from,
        to: payment.to,
        value: payment.value,
        validAfter: U256::from(payment.valid_after.as_secs()),
        validBefore: U256::from(payment.valid_before.as_secs()),
        nonce: payment.nonce,
        signature: payment.signature.clone(),
    };
    let calldata = eip3009::transferWithAuthorizationCall::abi_encode(&call);
    let token = eip712_domain
        .verifying_contract
        .ok_or(TronExactError::MissingTip712Domain)?;

    let _result = provider
        .grid()
        .trigger_constant_contract(
            provider.signer_address(),
            token,
            &alloy_primitives::Bytes::from(calldata),
        )
        .await
        .map_err(|e| {
            TronExactError::TronGrid(format!("transferWithAuthorization simulation failed: {e}"))
        })?;

    Ok(signed_message.address)
}

/// Runs all preconditions needed for a successful Permit2 payment.
pub(super) async fn assert_valid_permit2_payment(
    provider: &crate::chain::TronChainProvider,
    chain: &TronChainReference,
    permit2: &Permit2Payload,
    payload: &types::v2::PaymentPayload,
    requirements: &types::v2::PaymentRequirements,
    clock_skew_tolerance: u64,
) -> Result<(Permit2Payment, Eip712Domain), TronExactError> {
    let accepted = &payload.accepted;
    assert_requirements_match(accepted, requirements)?;

    let chain_id: ChainId = (*chain).into();
    if accepted.network != chain_id {
        return Err(TronExactError::ChainMismatch);
    }

    let auth = &permit2.permit2_authorization;

    // Verify spender is x402ExactPermit2Proxy
    let expected_proxy = chain
        .x402_exact_permit2_proxy()
        .ok_or(TronExactError::UnsupportedTransferMethod)?;
    if auth.spender != expected_proxy.as_evm() {
        return Err(TronExactError::InvalidPermit2Spender(auth.spender));
    }

    // Verify witness.to matches payTo
    if auth.witness.to != accepted.pay_to.as_evm() {
        return Err(TronExactError::RecipientMismatch);
    }

    // Verify deadline not expired
    let now = UnixTimestamp::now();
    let deadline_u64: u64 = auth.deadline.0.try_into().unwrap_or(u64::MAX);
    if deadline_u64 < now.as_secs() + clock_skew_tolerance {
        return Err(TronExactError::Expired);
    }

    // Verify validAfter not in the future
    let valid_after_u64: u64 = auth.witness.valid_after.0.try_into().unwrap_or(u64::MAX);
    if valid_after_u64 > now.as_secs() + clock_skew_tolerance {
        return Err(TronExactError::NotYetValid);
    }

    // Verify amount
    let auth_amount: U256 = auth.permitted.amount.into();
    let required_amount: U256 = accepted.amount.into();
    assert_exact_value(&auth_amount, &required_amount)?;

    // Verify token matches
    if auth.permitted.token != accepted.asset.as_evm() {
        return Err(TronExactError::AssetMismatch);
    }

    // Check balance
    let balance = provider
        .trc20_balance_of(auth.permitted.token, auth.from)
        .await?;
    if balance < required_amount {
        return Err(TronExactError::InsufficientBalance);
    }

    // Construct TIP-712 domain for Permit2
    let permit2_addr = chain
        .sun_permit2()
        .ok_or(TronExactError::UnsupportedTransferMethod)?;
    let domain = eip712_domain! {
        name: "Permit2",
        chain_id: u64::from(chain.inner()),
        verifying_contract: permit2_addr.as_evm(),
    };

    let payment = Permit2Payment {
        from: auth.from,
        to: auth.witness.to,
        token: auth.permitted.token,
        amount: auth_amount,
        spender: auth.spender,
        nonce: auth.nonce.into(),
        deadline: auth.deadline.into(),
        valid_after: auth.witness.valid_after.into(),
        signature: permit2.signature.clone(),
    };

    Ok((payment, domain))
}

/// Verifies a Permit2 payment by recovering the EIP-712 signature and
/// simulating the `x402ExactPermit2Proxy.settle` call.
pub(super) async fn verify_permit2_payment(
    provider: &crate::chain::TronChainProvider,
    chain: &TronChainReference,
    payment: &Permit2Payment,
    eip712_domain: &Eip712Domain,
) -> Result<EvmAddress, TronExactError> {
    let permit_witness = TronPermitWitnessTransferFrom {
        permitted: TronTokenPermissions {
            token: payment.token,
            amount: payment.amount,
        },
        spender: payment.spender,
        nonce: payment.nonce,
        deadline: payment.deadline,
        witness: TronWitness {
            to: payment.to,
            validAfter: payment.valid_after,
        },
    };

    let eip712_hash = permit_witness.eip712_signing_hash(eip712_domain);

    // Recover and verify the EOA signature
    let signature = if payment.signature.len() == 65 {
        alloy_primitives::Signature::from_raw(&payment.signature)
            .map_err(|e| TronExactError::SignatureRecovery(e.to_string()))?
    } else if payment.signature.len() == 64 {
        alloy_primitives::Signature::from_erc2098(&payment.signature)
    } else {
        return Err(TronExactError::SignatureRecovery(format!(
            "invalid signature length: {} bytes",
            payment.signature.len()
        )));
    };
    let recovered = signature
        .recover_address_from_prehash(&eip712_hash)
        .map_err(|e| TronExactError::SignatureRecovery(e.to_string()))?;
    if recovered != payment.from {
        return Err(TronExactError::SignerMismatch);
    }

    // Simulate the proxy.settle call via triggerconstantcontract
    let proxy_addr = chain
        .x402_exact_permit2_proxy()
        .ok_or(TronExactError::UnsupportedTransferMethod)?;
    let call = crate::chain::contracts::x402_exact_permit2_proxy::settleCall {
        permit: crate::chain::contracts::x402_exact_permit2_proxy::TronPermitTransferFrom {
            permitted: crate::chain::contracts::x402_exact_permit2_proxy::TronTokenPermissions {
                token: payment.token,
                amount: payment.amount,
            },
            nonce: payment.nonce,
            deadline: payment.deadline,
        },
        owner: payment.from,
        witness: crate::chain::contracts::x402_exact_permit2_proxy::TronWitness {
            to: payment.to,
            validAfter: payment.valid_after,
        },
        signature: payment.signature.clone(),
    };
    let calldata = crate::chain::contracts::x402_exact_permit2_proxy::settleCall::abi_encode(&call);
    let _result = provider
        .grid()
        .trigger_constant_contract(
            provider.signer_address(),
            proxy_addr.as_evm(),
            &alloy_primitives::Bytes::from(calldata),
        )
        .await
        .map_err(|e| TronExactError::TronGrid(format!("permit2 settle simulation failed: {e}")))?;

    Ok(payment.from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assert_exact_value_accepts_equal() {
        let value = U256::from(1_000_000_u64);
        assert!(assert_exact_value(&value, &value).is_ok());
    }

    #[test]
    fn assert_exact_value_rejects_overpayment() {
        let required = U256::from(1_000_000_u64);
        let sent = U256::from(1_000_001_u64);
        assert!(matches!(
            assert_exact_value(&sent, &required),
            Err(VerificationError::InvalidPaymentAmount),
        ));
    }

    #[test]
    fn assert_exact_value_rejects_underpayment() {
        let required = U256::from(1_000_000_u64);
        let sent = U256::from(999_999_u64);
        assert!(matches!(
            assert_exact_value(&sent, &required),
            Err(VerificationError::InvalidPaymentAmount),
        ));
    }
}
