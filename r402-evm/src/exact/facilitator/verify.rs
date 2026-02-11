//! Payment verification logic for the EIP-155 exact scheme.
//!
//! Contains precondition checks (time, domain, balance, value) and the
//! composite [`verify_payment`] function that ties signature verification
//! to an on-chain simulation.

use alloy_primitives::{Address, U256};
use alloy_provider::Provider;
use alloy_sol_types::SolStruct;
use alloy_sol_types::{Eip712Domain, eip712_domain};
use r402::chain::ChainId;
use r402::proto::PaymentVerificationError;
use r402::proto::UnixTimestamp;
#[cfg(feature = "telemetry")]
use tracing::instrument;

use super::Eip3009Payment;
use super::Permit2Payment;
use super::VALIDATOR_ADDRESS;
use super::contract::{IEIP3009, IERC20, Validator6492};
use super::error::Eip155ExactError;
use super::settle::{TransferWithAuthorization0Call, TransferWithAuthorization1Call};
use super::signature::{SignedMessage, StructuredSignature};
use crate::chain::Eip155ChainReference;
use crate::exact::Eip3009Payload;
use crate::exact::PaymentRequirementsExtra;
use crate::exact::PermitWitnessTransferFrom;
use crate::exact::types;
use crate::exact::types::TokenPermissions as SolTokenPermissions;
use crate::exact::types::Witness as SolWitness;
use crate::exact::{PERMIT2_ADDRESS, PERMIT2_DEADLINE_BUFFER, X402_EXACT_PERMIT2_PROXY};

/// Awaits a future, optionally instrumenting it with a tracing span.
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

/// Runs all preconditions needed for a successful EIP-3009 payment.
#[cfg_attr(feature = "telemetry", instrument(skip_all, err))]
pub(super) async fn assert_valid_payment<P: Provider>(
    provider: P,
    chain: &Eip155ChainReference,
    eip3009: &Eip3009Payload,
    payload: &types::v2::PaymentPayload,
    requirements: &types::v2::PaymentRequirements,
) -> Result<(IEIP3009::IEIP3009Instance<P>, Eip3009Payment, Eip712Domain), Eip155ExactError> {
    let accepted = &payload.accepted;
    if accepted != requirements {
        return Err(PaymentVerificationError::AcceptedRequirementsMismatch.into());
    }

    let chain_id: ChainId = chain.into();
    let payload_chain_id = &accepted.network;
    if payload_chain_id != &chain_id {
        return Err(PaymentVerificationError::ChainIdMismatch.into());
    }
    let authorization = &eip3009.authorization;
    if authorization.to != accepted.pay_to {
        return Err(PaymentVerificationError::RecipientMismatch.into());
    }
    let valid_after = authorization.valid_after;
    let valid_before = authorization.valid_before;
    assert_time(valid_after, valid_before)?;
    let asset_address = accepted.asset;
    let contract = IEIP3009::new(asset_address.into(), provider);

    let domain = assert_domain(chain, &contract, &asset_address.into(), &accepted.extra).await?;

    let amount_required = accepted.amount;
    assert_enough_balance(&contract, &authorization.from, amount_required.into()).await?;
    assert_enough_value(&authorization.value.into(), &amount_required.into())?;

    let payment = Eip3009Payment {
        from: authorization.from,
        to: authorization.to,
        value: authorization.value.into(),
        valid_after: authorization.valid_after,
        valid_before: authorization.valid_before,
        nonce: authorization.nonce,
        signature: eip3009.signature.clone(),
    };

    Ok((contract, payment, domain))
}

/// Validates that the current time is within the `validAfter` and `validBefore` bounds.
///
/// Adds a 6-second grace buffer when checking expiration to account for latency.
///
/// # Errors
///
/// Returns [`PaymentVerificationError::Expired`] or [`PaymentVerificationError::Early`].
#[cfg_attr(feature = "telemetry", instrument(skip_all, err))]
pub fn assert_time(
    valid_after: UnixTimestamp,
    valid_before: UnixTimestamp,
) -> Result<(), PaymentVerificationError> {
    let now = UnixTimestamp::now();
    if valid_before < now + 6 {
        return Err(PaymentVerificationError::Expired);
    }
    if valid_after > now {
        return Err(PaymentVerificationError::Early);
    }
    Ok(())
}

/// Constructs the correct EIP-712 domain for signature verification.
///
/// # Errors
///
/// Returns [`Eip155ExactError`] if on-chain name/version queries fail.
#[cfg_attr(feature = "telemetry", instrument(skip_all, err, fields(
    network = %chain.as_chain_id(),
    asset = %asset_address
)))]
pub async fn assert_domain<P: Provider>(
    chain: &Eip155ChainReference,
    token_contract: &IEIP3009::IEIP3009Instance<P>,
    asset_address: &Address,
    extra: &Option<PaymentRequirementsExtra>,
) -> Result<Eip712Domain, Eip155ExactError> {
    let name = extra.as_ref().map(|extra| extra.name.clone());
    let name = if let Some(name) = name {
        name
    } else {
        let name_b = token_contract.name();
        let name_fut = name_b.call().into_future();
        traced!(
            name_fut,
            tracing::info_span!("fetch_eip712_name", otel.kind = "client")
        )?
    };
    let version = extra.as_ref().map(|extra| extra.version.clone());
    let version = if let Some(version) = version {
        version
    } else {
        let version_b = token_contract.version();
        let version_fut = version_b.call().into_future();
        traced!(
            version_fut,
            tracing::info_span!("fetch_eip712_version", otel.kind = "client")
        )?
    };
    let domain = eip712_domain! {
        name: name,
        version: version,
        chain_id: chain.inner(),
        verifying_contract: *asset_address,
    };
    Ok(domain)
}

/// Checks if the payer has enough on-chain token balance to meet the `maxAmountRequired`.
///
/// # Errors
///
/// Returns [`Eip155ExactError`] if the balance query fails or funds are insufficient.
#[cfg_attr(feature = "telemetry", instrument(skip_all, err, fields(
    sender = %sender,
    max_required = %max_amount_required,
    token_contract = %ieip3009_token_contract.address()
)))]
pub async fn assert_enough_balance<P: Provider>(
    ieip3009_token_contract: &IEIP3009::IEIP3009Instance<P>,
    sender: &Address,
    max_amount_required: U256,
) -> Result<(), Eip155ExactError> {
    let balance_of = ieip3009_token_contract.balanceOf(*sender);
    let balance_fut = balance_of.call().into_future();
    let balance = traced!(
        balance_fut,
        tracing::info_span!(
            "fetch_token_balance",
            token_contract = %ieip3009_token_contract.address(),
            sender = %sender,
            otel.kind = "client"
        )
    )?;

    if balance < max_amount_required {
        Err(PaymentVerificationError::InsufficientFunds.into())
    } else {
        Ok(())
    }
}

/// Verifies that the declared `value` in the payload is sufficient for the required amount.
///
/// # Errors
///
/// Returns [`PaymentVerificationError::InvalidPaymentAmount`] if value is too low.
#[cfg_attr(feature = "telemetry", instrument(skip_all, err, fields(
    sent = %sent,
    max_amount_required = %max_amount_required
)))]
pub fn assert_enough_value(
    sent: &U256,
    max_amount_required: &U256,
) -> Result<(), PaymentVerificationError> {
    if sent < max_amount_required {
        Err(PaymentVerificationError::InvalidPaymentAmount)
    } else {
        Ok(())
    }
}

/// Verifies a payment by checking the signature and simulating the transfer call.
///
/// # Errors
///
/// Returns [`Eip155ExactError`] if signature verification or simulation fails.
pub async fn verify_payment<P: Provider>(
    provider: &P,
    contract: &IEIP3009::IEIP3009Instance<&P>,
    payment: &Eip3009Payment,
    eip712_domain: &Eip712Domain,
) -> Result<Address, Eip155ExactError> {
    let signed_message = SignedMessage::extract(payment, eip712_domain)?;

    let payer = signed_message.address;
    let hash = signed_message.hash;
    match signed_message.signature {
        StructuredSignature::EIP6492 {
            factory: _,
            factory_calldata: _,
            inner,
            original,
        } => {
            let validator6492 = Validator6492::new(VALIDATOR_ADDRESS, &provider);
            let is_valid_signature_call =
                validator6492.isValidSigWithSideEffects(payer, hash, original);
            let transfer_call = TransferWithAuthorization0Call::new(contract, payment, inner);
            let transfer_call = transfer_call.0;
            let aggregate3 = provider
                .multicall()
                .add(is_valid_signature_call)
                .add(transfer_call.tx);
            let aggregate3_call = aggregate3.aggregate3();
            let (is_valid_signature_result, transfer_result) = traced!(
                aggregate3_call,
                tracing::info_span!("call_transferWithAuthorization_0",
                    from = %transfer_call.from,
                    to = %transfer_call.to,
                    value = %transfer_call.value,
                    valid_after = %transfer_call.valid_after,
                    valid_before = %transfer_call.valid_before,
                    nonce = %transfer_call.nonce,
                    signature = %transfer_call.signature,
                    token_contract = %transfer_call.contract_address,
                    otel.kind = "client",
                )
            )?;
            let is_valid_signature_result = is_valid_signature_result
                .map_err(|e| PaymentVerificationError::InvalidSignature(e.to_string()))?;
            if !is_valid_signature_result {
                return Err(PaymentVerificationError::InvalidSignature(
                    "Chain reported signature to be invalid".to_string(),
                )
                .into());
            }
            transfer_result
                .map_err(|e| PaymentVerificationError::TransactionSimulation(e.to_string()))?;
        }
        StructuredSignature::EIP1271(signature) => {
            let transfer_call = TransferWithAuthorization0Call::new(contract, payment, signature);
            let transfer_call = transfer_call.0;
            let transfer_call_fut = transfer_call.tx.call().into_future();
            traced!(
                transfer_call_fut,
                tracing::info_span!("call_transferWithAuthorization_0",
                    from = %transfer_call.from,
                    to = %transfer_call.to,
                    value = %transfer_call.value,
                    valid_after = %transfer_call.valid_after,
                    valid_before = %transfer_call.valid_before,
                    nonce = %transfer_call.nonce,
                    signature = %transfer_call.signature,
                    token_contract = %transfer_call.contract_address,
                    otel.kind = "client",
                )
            )?;
        }
        StructuredSignature::EOA(signature) => {
            let transfer_call = TransferWithAuthorization1Call::new(contract, payment, signature);
            let transfer_call = transfer_call.0;
            let transfer_call_fut = transfer_call.tx.call().into_future();
            traced!(
                transfer_call_fut,
                tracing::info_span!("call_transferWithAuthorization_1",
                    from = %transfer_call.from,
                    to = %transfer_call.to,
                    value = %transfer_call.value,
                    valid_after = %transfer_call.valid_after,
                    valid_before = %transfer_call.valid_before,
                    nonce = %transfer_call.nonce,
                    signature = %transfer_call.signature,
                    token_contract = %transfer_call.contract_address,
                    otel.kind = "client",
                )
            )?;
        }
    }

    Ok(payer)
}

/// Runs all preconditions needed for a successful Permit2 payment.
///
/// Validates the Permit2 authorization parameters against the payment requirements,
/// following the same checks as the official Go SDK's `VerifyPermit2`:
/// spender, recipient, deadline, validAfter, amount, and token.
#[cfg_attr(feature = "telemetry", instrument(skip_all, err))]
pub(super) async fn assert_valid_permit2_payment<P: Provider>(
    provider: P,
    chain: &Eip155ChainReference,
    permit2: &crate::exact::Permit2Payload,
    payload: &types::v2::PaymentPayload,
    requirements: &types::v2::PaymentRequirements,
) -> Result<(IERC20::IERC20Instance<P>, Permit2Payment, Eip712Domain), Eip155ExactError> {
    let accepted = &payload.accepted;
    if accepted != requirements {
        return Err(PaymentVerificationError::AcceptedRequirementsMismatch.into());
    }

    let chain_id: ChainId = chain.into();
    if accepted.network != chain_id {
        return Err(PaymentVerificationError::ChainIdMismatch.into());
    }

    let auth = &permit2.permit2_authorization;

    // Verify spender is x402ExactPermit2Proxy
    if auth.spender != X402_EXACT_PERMIT2_PROXY {
        return Err(PaymentVerificationError::InvalidSignature(
            "invalid Permit2 spender: must be x402ExactPermit2Proxy".into(),
        )
        .into());
    }

    // Verify witness.to matches payTo
    if auth.witness.to != Address::from(accepted.pay_to) {
        return Err(PaymentVerificationError::RecipientMismatch.into());
    }

    // Parse and verify deadline not expired (with buffer for block time)
    let now = UnixTimestamp::now();
    let deadline_u64: u64 = auth.deadline.0.try_into().unwrap_or(u64::MAX);
    let deadline_threshold = now.as_secs() + PERMIT2_DEADLINE_BUFFER;
    if deadline_u64 < deadline_threshold {
        return Err(PaymentVerificationError::Expired.into());
    }

    // Parse and verify validAfter is not in the future
    let valid_after_u64: u64 = auth.witness.valid_after.0.try_into().unwrap_or(u64::MAX);
    if valid_after_u64 > now.as_secs() {
        return Err(PaymentVerificationError::Early.into());
    }

    // Verify amount is sufficient
    let auth_amount: U256 = auth.permitted.amount.into();
    let required_amount: U256 = accepted.amount.into();
    assert_enough_value(&auth_amount, &required_amount)?;

    // Verify token matches
    if auth.permitted.token != Address::from(accepted.asset) {
        return Err(PaymentVerificationError::InvalidPaymentAmount.into());
    }

    let token_address: Address = accepted.asset.into();
    let erc20 = IERC20::new(token_address, provider);

    // Check Permit2 allowance (non-fatal if RPC fails, matching Go SDK behavior)
    let allowance_result = erc20.allowance(auth.from, PERMIT2_ADDRESS).call().await;
    if let Ok(allowance) = allowance_result
        && allowance < required_amount
    {
        return Err(PaymentVerificationError::InsufficientFunds.into());
    }

    // Check balance
    let balance_result = erc20.balanceOf(auth.from).call().await;
    if let Ok(balance) = balance_result
        && balance < required_amount
    {
        return Err(PaymentVerificationError::InsufficientFunds.into());
    }

    // Construct EIP-712 domain for Permit2 (name = "Permit2", no version)
    let domain = eip712_domain! {
        name: "Permit2",
        chain_id: chain.inner(),
        verifying_contract: PERMIT2_ADDRESS,
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
        extra: auth.witness.extra.clone(),
        signature: permit2.signature.clone(),
    };

    Ok((erc20, payment, domain))
}

/// Verifies a Permit2 payment by checking the EIP-712 signature.
///
/// Reconstructs the `PermitWitnessTransferFrom` typed data, computes the
/// EIP-712 signing hash, and verifies the signature using the EIP-6492
/// universal validator (supporting EOA, EIP-1271, and counterfactual wallets).
///
/// # Errors
///
/// Returns [`Eip155ExactError`] if signature verification fails.
pub async fn verify_permit2_payment<P: Provider>(
    provider: &P,
    payment: &Permit2Payment,
    eip712_domain: &Eip712Domain,
) -> Result<Address, Eip155ExactError> {
    let permit_witness = PermitWitnessTransferFrom {
        permitted: SolTokenPermissions {
            token: payment.token,
            amount: payment.amount,
        },
        spender: payment.spender,
        nonce: payment.nonce,
        deadline: payment.deadline,
        witness: SolWitness {
            to: payment.to,
            validAfter: payment.valid_after,
            extra: payment.extra.clone(),
        },
    };

    let eip712_hash = permit_witness.eip712_signing_hash(eip712_domain);
    let payer = payment.from;
    let signature_bytes = payment.signature.clone();

    // Use universal signature verification (EIP-6492 validator)
    let validator6492 = Validator6492::new(VALIDATOR_ADDRESS, provider);
    let is_valid_call =
        validator6492.isValidSigWithSideEffects(payer, eip712_hash, signature_bytes);
    let is_valid_fut = is_valid_call.call().into_future();
    let is_valid = traced!(
        is_valid_fut,
        tracing::info_span!("verify_permit2_signature",
            from = %payer,
            token = %payment.token,
            amount = %payment.amount,
            otel.kind = "client",
        )
    )
    .map_err(|e| PaymentVerificationError::InvalidSignature(e.to_string()))?;

    if !is_valid {
        return Err(
            PaymentVerificationError::InvalidSignature("invalid Permit2 signature".into()).into(),
        );
    }

    Ok(payer)
}
