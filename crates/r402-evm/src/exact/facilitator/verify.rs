//! Payment verification logic for the EIP-155 exact scheme.
//!
//! Contains precondition checks (time, domain, balance, value) and the
//! composite [`verify_payment`] function that ties signature verification
//! to an on-chain simulation.

use std::future::IntoFuture;

use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_provider::Provider;
use alloy_sol_types::{Eip712Domain, SolStruct, eip712_domain};
use r402_protocol::error::VerificationError;
use r402_protocol::network::ChainId;
use r402_protocol::payment::Extensions;
use r402_protocol::payment::UnixTimestamp;
#[cfg(feature = "telemetry")]
use tracing::instrument;

use super::contract::{IEIP3009, IX402Permit2Proxy};
use super::eip6492::{ASSET_NOT_DEPLOYED, FACTORY_NOT_ALLOWED, deny_undeployed_factory};
use super::settle::{
    TransferWithAuthorization0Call, TransferWithAuthorization1Call, build_eip2612_abi,
};
use super::signature::SignedMessage;
use super::{Eip3009Payment, Permit2Payment};
use crate::asset::VALIDATOR_ADDRESS;
use crate::chain::Eip155ChainReference;
use crate::chain::contracts::{IERC20, Validator6492};
use crate::eip2612::Eip2612SignedPermit;
use crate::erc20_approval::Erc20ApprovalGasSponsoringInfo;
use crate::error::Eip155ExactError;
use crate::exact::payload::{TokenPermissions as SolTokenPermissions, Witness as SolWitness};
use crate::exact::{
    Eip3009Payload, PaymentRequirementsExtra, PermitWitnessTransferFrom, X402_EXACT_PERMIT2_PROXY,
    payload,
};
use crate::permit2::PERMIT2_ADDRESS;
use crate::signature::{ClassifiedSignature, assert_time, classify_with_code};

/// Runs all preconditions needed for a successful EIP-3009 payment.
#[cfg_attr(feature = "telemetry", instrument(skip_all, err))]
pub(super) async fn assert_valid_payment<P: Provider>(
    provider: P,
    chain: &Eip155ChainReference,
    eip3009: &Eip3009Payload,
    payload: &payload::v2::PaymentPayload,
    requirements: &payload::v2::PaymentRequirements,
    clock_skew_tolerance: u64,
) -> Result<(IEIP3009::IEIP3009Instance<P>, Eip3009Payment, Eip712Domain), Eip155ExactError> {
    let accepted = &payload.accepted;
    assert_requirements_match(accepted, requirements)?;

    let chain_id: ChainId = chain.into();
    let payload_chain_id = &accepted.network;
    if payload_chain_id != &chain_id {
        return Err(VerificationError::ChainIdMismatch.into());
    }
    let authorization = &eip3009.authorization;
    if authorization.to != accepted.pay_to {
        return Err(VerificationError::RecipientMismatch.into());
    }
    let valid_after = authorization.valid_after;
    let valid_before = authorization.valid_before;
    assert_time(valid_after, valid_before, clock_skew_tolerance)?;
    let asset_address = accepted.asset;
    let contract = IEIP3009::new(asset_address.into(), provider);

    let amount_required = accepted.amount;

    let asset_addr: Address = asset_address.into();
    let (domain, nonce, balance, code) = tokio::join!(
        assert_domain(chain, &contract, &asset_addr, accepted.extra.as_ref()),
        assert_nonce_unused(&contract, &authorization.from, &authorization.nonce),
        assert_enough_balance(&contract, &authorization.from, amount_required.into()),
        contract.provider().get_code_at(asset_addr),
    );
    let code = code.map_err(Eip155ExactError::from)?;
    if code.is_empty() {
        return Err(VerificationError::from_wire(ASSET_NOT_DEPLOYED).into());
    }
    let domain = domain?;
    let () = nonce?;
    let () = balance?;
    assert_exact_value(&authorization.value.into(), &amount_required.into())?;

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

/// Validates that `accepted` satisfies `requirements` via
/// [`r402_protocol::payment::PaymentRequirements::matches_payload_accepted`].
///
/// # Errors
///
/// Returns [`VerificationError::AcceptedRequirementsMismatch`] on mismatch.
pub(super) fn assert_requirements_match(
    accepted: &payload::v2::PaymentRequirements,
    requirements: &payload::v2::PaymentRequirements,
) -> Result<(), VerificationError> {
    if requirements.matches_payload_accepted(accepted) {
        Ok(())
    } else {
        Err(VerificationError::AcceptedRequirementsMismatch)
    }
}

/// Checks whether the EIP-3009 authorization nonce has already been used on-chain.
///
/// Calls `authorizationState(address, bytes32)` on the token contract. If the
/// nonce is already consumed, the payment is a replay and must be rejected.
///
/// # Errors
///
/// Returns [`Eip155ExactError`] if the RPC call fails or the nonce is already used.
#[cfg_attr(feature = "telemetry", instrument(skip_all, err, fields(
    from = %authorizer,
    nonce = %nonce
)))]
pub(super) async fn assert_nonce_unused<P: Provider>(
    contract: &IEIP3009::IEIP3009Instance<P>,
    authorizer: &Address,
    nonce: &B256,
) -> Result<(), Eip155ExactError> {
    let call = contract.authorizationState(*authorizer, *nonce);
    let used_fut = call.call().into_future();
    let used = traced!(
        used_fut,
        tracing::info_span!("check_authorization_state", otel.kind = "client")
    )?;
    if used {
        return Err(VerificationError::NonceAlreadyUsed.into());
    }
    Ok(())
}

/// Validates that the current time is within the `validAfter` and `validBefore` bounds.
///
/// Constructs the correct EIP-712 domain for signature verification.
///
/// # Errors
///
/// Returns [`Eip155ExactError`] if on-chain name/version queries fail.
#[cfg_attr(feature = "telemetry", instrument(skip_all, err, fields(
    network = %chain.as_chain_id(),
    asset = %asset_address
)))]
pub(super) async fn assert_domain<P: Provider>(
    chain: &Eip155ChainReference,
    token_contract: &IEIP3009::IEIP3009Instance<P>,
    asset_address: &Address,
    extra: Option<&PaymentRequirementsExtra>,
) -> Result<Eip712Domain, Eip155ExactError> {
    let name = extra.map(|extra| extra.name.clone());
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
    let version = extra.map(|extra| extra.version.clone());
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
pub(super) async fn assert_enough_balance<P: Provider>(
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
        Err(VerificationError::InsufficientFunds.into())
    } else {
        Ok(())
    }
}

/// Asserts that the declared payment amount equals the required amount exactly.
///
/// x402 v2 §Payment Scheme "exact" mandates buyers transfer **exactly**
/// `maxAmountRequired`. Over- or under-payment is rejected. The previous
/// `>=` comparison is a protocol violation that was replaced in v0.14.0.
///
/// # Errors
///
/// Returns [`VerificationError::InvalidPaymentAmount`] when `sent != required`.
#[cfg_attr(feature = "telemetry", instrument(skip_all, err, fields(
    sent = %sent,
    required = %required,
)))]
pub(super) fn assert_exact_value(sent: &U256, required: &U256) -> Result<(), VerificationError> {
    if sent == required {
        Ok(())
    } else {
        Err(VerificationError::InvalidPaymentAmount)
    }
}

/// Verifies a payment by checking the signature and simulating the transfer call.
///
/// # Errors
///
/// Returns [`Eip155ExactError`] if signature verification or simulation fails.
pub(super) async fn verify_payment<P: Provider>(
    provider: &P,
    contract: &IEIP3009::IEIP3009Instance<&P>,
    payment: &Eip3009Payment,
    eip712_domain: &Eip712Domain,
    eip6492_allowed_factories: &[Address],
) -> Result<Address, Eip155ExactError> {
    let signed_message = SignedMessage::extract(payment, eip712_domain)?;

    let payer = signed_message.address;
    let hash = signed_message.hash;
    let classified = classify_with_code(provider, payer, signed_message.signature, &hash).await?;
    match classified {
        ClassifiedSignature::EIP6492 {
            factory,
            factory_calldata: _,
            inner,
            original,
        } => {
            simulate_eip6492_transfer(
                provider,
                contract,
                payment,
                payer,
                hash,
                Eip6492Call {
                    factory,
                    inner,
                    original,
                },
                eip6492_allowed_factories,
            )
            .await?;
        }
        ClassifiedSignature::EIP1271(signature) => {
            let transfer_call = TransferWithAuthorization0Call::new(contract, payment, signature);
            let transfer_call = transfer_call.0;
            let transfer_call_fut = transfer_call.tx.call().into_future();
            traced!(
                transfer_call_fut,
                transfer_span!("call_transferWithAuthorization_0", transfer_call)
            )?;
        }
        ClassifiedSignature::Eoa(signature) => {
            let transfer_call = TransferWithAuthorization1Call::new(contract, payment, signature);
            let transfer_call = transfer_call.0;
            let transfer_call_fut = transfer_call.tx.call().into_future();
            traced!(
                transfer_call_fut,
                transfer_span!("call_transferWithAuthorization_1", transfer_call)
            )?;
        }
    }

    Ok(payer)
}

struct Eip6492Call {
    factory: Address,
    inner: Bytes,
    original: Bytes,
}

async fn simulate_eip6492_transfer<P: Provider>(
    provider: &P,
    contract: &IEIP3009::IEIP3009Instance<&P>,
    payment: &Eip3009Payment,
    payer: Address,
    hash: B256,
    call: Eip6492Call,
    eip6492_allowed_factories: &[Address],
) -> Result<(), Eip155ExactError> {
    let payer_code = provider
        .get_code_at(payer)
        .await
        .unwrap_or_else(|_| Bytes::new());
    if deny_undeployed_factory(call.factory, payer_code.as_ref(), eip6492_allowed_factories) {
        #[cfg(feature = "telemetry")]
        tracing::warn!(factory = %call.factory, "eip6492_factory_not_allowed");
        return Err(VerificationError::from_wire(FACTORY_NOT_ALLOWED).into());
    }
    let validator6492 = Validator6492::new(VALIDATOR_ADDRESS, &provider);
    let is_valid_signature_call =
        validator6492.isValidSigWithSideEffects(payer, hash, call.original);
    let transfer_call = TransferWithAuthorization0Call::new(contract, payment, call.inner);
    let transfer_call = transfer_call.0;
    let aggregate3 = provider
        .multicall()
        .add(is_valid_signature_call)
        .add(transfer_call.tx);
    let aggregate3_call = aggregate3.aggregate3();
    let (is_valid_signature_result, transfer_result) = traced!(
        aggregate3_call,
        transfer_span!("call_transferWithAuthorization_0", transfer_call)
    )?;
    let is_valid_signature_result = is_valid_signature_result
        .map_err(|e| VerificationError::InvalidSignature(e.to_string()))?;
    if !is_valid_signature_result {
        return Err(VerificationError::InvalidSignature(
            "Chain reported signature to be invalid".to_owned(),
        )
        .into());
    }
    transfer_result.map_err(|e| VerificationError::SimulationFailed(e.to_string()))?;
    Ok(())
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
    payload: &payload::v2::PaymentPayload,
    requirements: &payload::v2::PaymentRequirements,
    clock_skew_tolerance: u64,
    erc20_approval_enabled: bool,
) -> Result<(IERC20::IERC20Instance<P>, Permit2Payment, Eip712Domain), Eip155ExactError> {
    let accepted = &payload.accepted;
    assert_requirements_match(accepted, requirements)?;

    let chain_id: ChainId = chain.into();
    if accepted.network != chain_id {
        return Err(VerificationError::ChainIdMismatch.into());
    }

    let auth = &permit2.permit2_authorization;

    // Verify spender is x402ExactPermit2Proxy
    if auth.spender != X402_EXACT_PERMIT2_PROXY {
        return Err(VerificationError::InvalidSignature(
            "invalid Permit2 spender: must be x402ExactPermit2Proxy".into(),
        )
        .into());
    }

    // Verify witness.to matches payTo
    if auth.witness.to != Address::from(accepted.pay_to) {
        return Err(VerificationError::RecipientMismatch.into());
    }

    // Parse and verify deadline not expired (with clock skew tolerance)
    let now = UnixTimestamp::now();
    let deadline_u64: u64 = auth.deadline.0.try_into().unwrap_or(u64::MAX);
    let deadline_threshold = now.as_secs() + clock_skew_tolerance;
    if deadline_u64 < deadline_threshold {
        return Err(VerificationError::Expired.into());
    }

    // Parse and verify validAfter is not in the future (with clock skew tolerance)
    let valid_after_u64: u64 = auth.witness.valid_after.0.try_into().unwrap_or(u64::MAX);
    if valid_after_u64 > now.as_secs() + clock_skew_tolerance {
        return Err(VerificationError::Early.into());
    }

    // Verify amount is sufficient
    let auth_amount: U256 = auth.permitted.amount.into();
    let required_amount: U256 = accepted.amount.into();
    assert_exact_value(&auth_amount, &required_amount)?;

    // Verify token matches
    if auth.permitted.token != Address::from(accepted.asset) {
        return Err(VerificationError::AssetMismatch.into());
    }

    let token_address: Address = accepted.asset.into();
    let erc20 = IERC20::new(token_address, provider);

    let allowance_call = erc20.allowance(auth.from, PERMIT2_ADDRESS);
    let balance_call = erc20.balanceOf(auth.from);
    let (allowance_result, balance_result, code_result) = tokio::join!(
        allowance_call.call(),
        balance_call.call(),
        erc20.provider().get_code_at(token_address),
    );

    let code = code_result.map_err(Eip155ExactError::from)?;
    if code.is_empty() {
        return Err(VerificationError::from_wire(ASSET_NOT_DEPLOYED).into());
    }

    let allowance_rpc_failed = allowance_result.is_err();
    let allowance_for_gate = allowance_result.as_ref().map(|v| *v).map_err(|_| ());
    match permit2_allowance_gate(allowance_for_gate, required_amount, false, false) {
        Ok(()) => {}
        Err(VerificationError::Permit2AllowanceRequired) => {
            if !permit2_extension_covers_allowance(
                &payload.extensions,
                auth.from,
                token_address,
                clock_skew_tolerance,
                erc20_approval_enabled,
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

    // Construct EIP-712 domain for Permit2 (name = "Permit2", no version)
    let domain = eip712_domain! {
        name: "Permit2",
        chain_id: chain.inner(),
        verifying_contract: PERMIT2_ADDRESS,
    };

    let Ok(eip2612) = Eip2612SignedPermit::from_extensions(&payload.extensions) else {
        return Err(VerificationError::from_wire("invalid_eip2612_extension_format").into());
    };
    if let Some(ref info) = eip2612 {
        validate_eip2612_permit_for_payment(info, auth.from, token_address, clock_skew_tolerance)?;
    }

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
        eip2612,
    };

    Ok((erc20, payment, domain))
}

/// Verifies a Permit2 payment by checking the EIP-712 signature **and**
/// simulating the on-chain `x402ExactPermit2Proxy.settle` call via
/// `eth_call` (Fix-7).
///
/// Signature validity alone does not guarantee the settlement transaction
/// will succeed: the buyer may have burned the nonce, revoked the
/// allowance, or set a short deadline that expires between verify and
/// settle. Running `eth_call` on the exact call that `settle_permit2_payment`
/// would submit catches all of these revert conditions before the
/// facilitator commits gas.
///
/// # Errors
///
/// Returns [`Eip155ExactError`] if signature verification or the
/// simulation call fails.
pub(super) async fn verify_permit2_payment<P: Provider>(
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
        },
    };

    let eip712_hash = permit_witness.eip712_signing_hash(eip712_domain);
    let payer = payment.from;
    let signature_bytes = payment.signature.clone();

    let validator6492 = Validator6492::new(VALIDATOR_ADDRESS, provider);
    let is_valid_call =
        validator6492.isValidSigWithSideEffects(payer, eip712_hash, signature_bytes.clone());
    let is_valid_fut = is_valid_call.call().into_future();
    let is_valid = traced!(
        is_valid_fut,
        tracing::info_span!(
            "verify_permit2_signature",
            from = %payer,
            token = %payment.token,
            amount = %payment.amount,
            otel.kind = "client",
        )
    )
    .map_err(|e| VerificationError::InvalidSignature(e.to_string()))?;

    if !is_valid {
        return Err(VerificationError::InvalidSignature("invalid Permit2 signature".into()).into());
    }

    // Simulate the exact proxy call settle would submit: settleWithPermit when
    // eip2612GasSponsoring is attached, otherwise settle.
    let proxy = IX402Permit2Proxy::new(X402_EXACT_PERMIT2_PROXY, provider);
    let permit = IX402Permit2Proxy::Permit {
        permitted: IX402Permit2Proxy::TokenPermissions {
            token: payment.token,
            amount: payment.amount,
        },
        nonce: payment.nonce,
        deadline: payment.deadline,
    };
    let witness = IX402Permit2Proxy::Witness {
        to: payment.to,
        validAfter: payment.valid_after,
    };
    if let Some(eip2612) = payment.eip2612.as_ref() {
        let settle_call = proxy.settleWithPermit(
            build_eip2612_abi(eip2612)?,
            permit,
            payment.from,
            witness,
            signature_bytes,
        );
        let settle_simulation = settle_call.call().into_future();
        traced!(
            settle_simulation,
            tracing::info_span!(
                "simulate_permit2_settle_with_permit",
                from = %payer,
                to = %payment.to,
                token = %payment.token,
                amount = %payment.amount,
                otel.kind = "client",
            )
        )
        .map_err(|e| VerificationError::SimulationFailed(e.to_string()))?;
    } else {
        let settle_call = proxy.settle(permit, payment.from, witness, signature_bytes);
        let settle_simulation = settle_call.call().into_future();
        traced!(
            settle_simulation,
            tracing::info_span!(
                "simulate_permit2_settle",
                from = %payer,
                to = %payment.to,
                token = %payment.token,
                amount = %payment.amount,
                otel.kind = "client",
            )
        )
        .map_err(|e| VerificationError::SimulationFailed(e.to_string()))?;
    }

    Ok(payer)
}

const APPROVE_SELECTOR: [u8; 4] = [0x09, 0x5e, 0xa7, 0xb3];

pub(crate) fn permit2_extension_covers_allowance(
    extensions: &Extensions,
    payer: Address,
    token: Address,
    clock_skew_tolerance: u64,
    erc20_approval_enabled: bool,
) -> Result<bool, Eip155ExactError> {
    match Eip2612SignedPermit::from_extensions(extensions) {
        Ok(Some(info)) => {
            validate_eip2612_permit_for_payment(&info, payer, token, clock_skew_tolerance)?;
            return Ok(true);
        }
        Ok(None) => {}
        Err(_) => {
            return Err(VerificationError::from_wire("invalid_eip2612_extension_format").into());
        }
    }
    if !erc20_approval_enabled {
        return Ok(false);
    }
    match Erc20ApprovalGasSponsoringInfo::from_extensions(extensions) {
        Ok(Some(info)) => {
            validate_erc20_approval_for_payment(&info, payer, token)?;
            Ok(true)
        }
        Ok(None) => Ok(false),
        Err(_) => {
            Err(VerificationError::from_wire("invalid_erc20_approval_extension_format").into())
        }
    }
}

fn validate_eip2612_permit_for_payment(
    info: &Eip2612SignedPermit,
    payer: Address,
    token: Address,
    clock_skew_tolerance: u64,
) -> Result<(), VerificationError> {
    if info.from != payer {
        return Err(VerificationError::from_wire("eip2612_from_mismatch"));
    }
    if info.asset != token {
        return Err(VerificationError::from_wire("eip2612_asset_mismatch"));
    }
    if info.spender != PERMIT2_ADDRESS {
        return Err(VerificationError::from_wire("eip2612_spender_not_permit2"));
    }
    let deadline: u64 = info.deadline.0.try_into().unwrap_or(0);
    let threshold = UnixTimestamp::now()
        .as_secs()
        .saturating_add(clock_skew_tolerance);
    if deadline < threshold {
        return Err(VerificationError::from_wire("eip2612_deadline_expired"));
    }
    Ok(())
}

fn validate_erc20_approval_for_payment(
    info: &Erc20ApprovalGasSponsoringInfo,
    payer: Address,
    token: Address,
) -> Result<(), VerificationError> {
    if info.from != payer {
        return Err(VerificationError::from_wire("erc20_approval_from_mismatch"));
    }
    if info.asset != token {
        return Err(VerificationError::from_wire(
            "erc20_approval_asset_mismatch",
        ));
    }
    if info.spender != PERMIT2_ADDRESS {
        return Err(VerificationError::from_wire(
            "erc20_approval_spender_not_permit2",
        ));
    }
    if info.version.is_empty() {
        return Err(VerificationError::from_wire(
            "invalid_erc20_approval_extension_format",
        ));
    }
    validate_erc20_approval_signed_tx(&info.signed_transaction, payer, token)
}

fn validate_erc20_approval_signed_tx(
    signed_transaction: &str,
    payer: Address,
    token: Address,
) -> Result<(), VerificationError> {
    use alloy_consensus::TxEnvelope;
    use alloy_consensus::transaction::{SignerRecoverable, Transaction};
    use alloy_eips::eip2718::Decodable2718;
    use alloy_primitives::hex;

    let hex_body = signed_transaction
        .strip_prefix("0x")
        .or_else(|| signed_transaction.strip_prefix("0X"))
        .unwrap_or(signed_transaction);
    let bytes = hex::decode(hex_body)
        .map_err(|_| VerificationError::from_wire("erc20_approval_tx_parse_failed"))?;
    let tx = TxEnvelope::decode_2718(&mut bytes.as_slice())
        .map_err(|_| VerificationError::from_wire("erc20_approval_tx_parse_failed"))?;
    let Some(to) = tx.to() else {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_wrong_target",
        ));
    };
    if to != token {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_wrong_target",
        ));
    }
    let data = tx.input();
    let Some(selector) = data.get(..4) else {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_wrong_selector",
        ));
    };
    if selector != APPROVE_SELECTOR {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_wrong_selector",
        ));
    }
    let Some(spender_word) = data.get(4..36) else {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_invalid_calldata",
        ));
    };
    let Some(spender_bytes) = spender_word.get(12..) else {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_invalid_calldata",
        ));
    };
    let Ok(spender) = Address::try_from(spender_bytes) else {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_invalid_calldata",
        ));
    };
    if spender != PERMIT2_ADDRESS {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_wrong_spender",
        ));
    }
    let recovered = tx
        .recover_signer()
        .map_err(|_| VerificationError::from_wire("erc20_approval_tx_invalid_signature"))?;
    if recovered != payer {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_signer_mismatch",
        ));
    }
    Ok(())
}

/// Fail-closed Permit2 allowance decision used by verify (and unit tests).
pub(crate) fn permit2_allowance_gate(
    allowance: Result<U256, ()>,
    required: U256,
    eip2612_ok: bool,
    erc20_ok: bool,
) -> Result<(), VerificationError> {
    match allowance {
        Ok(value) if value >= required => Ok(()),
        Ok(_) | Err(()) if eip2612_ok || erc20_ok => Ok(()),
        Ok(_) | Err(()) => Err(VerificationError::Permit2AllowanceRequired),
    }
}

#[cfg(test)]
mod tests {
    use r402_protocol::error::AsPaymentProblem;
    use r402_protocol::payment::ExtensionEntry;

    use super::*;
    use crate::erc20_approval::ERC20_APPROVAL_GAS_SPONSORING_KEY;

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

    fn sample_requirements(
        timeout: u64,
        extra: &serde_json::Value,
    ) -> payload::v2::PaymentRequirements {
        serde_json::from_value(serde_json::json!({
            "scheme": "exact",
            "network": "eip155:1",
            "amount": "1000000",
            "payTo": "0x0000000000000000000000000000000000000001",
            "maxTimeoutSeconds": timeout,
            "asset": "0x0000000000000000000000000000000000000002",
            "extra": extra,
        }))
        .unwrap()
    }

    #[test]
    fn assert_requirements_match_rejects_max_timeout_mismatch() {
        let extra = serde_json::json!({ "name": "USDC", "version": "2" });
        let requirements = sample_requirements(60, &extra);
        let accepted = sample_requirements(999, &extra);
        assert!(matches!(
            assert_requirements_match(&accepted, &requirements),
            Err(VerificationError::AcceptedRequirementsMismatch),
        ));
    }

    #[test]
    fn assert_requirements_match_rejects_extra_mismatch() {
        let requirements =
            sample_requirements(60, &serde_json::json!({ "name": "USDC", "version": "2" }));
        let accepted =
            sample_requirements(60, &serde_json::json!({ "name": "USDT", "version": "2" }));
        assert!(matches!(
            assert_requirements_match(&accepted, &requirements),
            Err(VerificationError::AcceptedRequirementsMismatch),
        ));
    }

    #[test]
    fn permit2_allowance_rpc_err_without_extension_is_412() {
        let required = U256::from(1_000_000_u64);
        assert!(matches!(
            permit2_allowance_gate(Err(()), required, false, false),
            Err(VerificationError::Permit2AllowanceRequired),
        ));
    }

    #[test]
    fn permit2_allowance_rpc_err_with_eip2612_continues() {
        let required = U256::from(1_000_000_u64);
        assert!(permit2_allowance_gate(Err(()), required, true, false).is_ok());
    }

    #[test]
    fn permit2_allowance_rpc_err_with_erc20_approval_continues() {
        let required = U256::from(1_000_000_u64);
        assert!(permit2_allowance_gate(Err(()), required, false, true).is_ok());
    }

    #[test]
    fn permit2_allowance_low_is_412() {
        let required = U256::from(1_000_000_u64);
        assert!(matches!(
            permit2_allowance_gate(Ok(U256::from(1_u64)), required, false, false),
            Err(VerificationError::Permit2AllowanceRequired),
        ));
    }

    #[test]
    fn permit2_allowance_sufficient_skips_extensions() {
        let required = U256::from(1_000_000_u64);
        assert!(permit2_allowance_gate(Ok(required), required, false, false).is_ok());
    }

    fn sign_approve_tx(
        signer: &alloy_signer_local::PrivateKeySigner,
        token: Address,
        spender: Address,
    ) -> String {
        use alloy_consensus::{SignableTransaction, TxEip1559, TxEnvelope};
        use alloy_eips::eip2718::Encodable2718;
        use alloy_eips::eip2930::AccessList;
        use alloy_network::TxSignerSync;
        use alloy_primitives::{TxKind, hex};

        let mut calldata = Vec::from(APPROVE_SELECTOR);
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(spender.as_slice());
        calldata.extend_from_slice(&[0u8; 32]);
        let mut tx = TxEip1559 {
            chain_id: 8453,
            nonce: 0,
            gas_limit: 80_000,
            max_fee_per_gas: 1_000_000_000,
            max_priority_fee_per_gas: 1_000_000,
            to: TxKind::Call(token),
            value: U256::ZERO,
            access_list: AccessList::default(),
            input: calldata.into(),
        };
        let sig = signer.sign_transaction_sync(&mut tx).expect("sign tx");
        let encoded = TxEnvelope::Eip1559(tx.into_signed(sig)).encoded_2718();
        format!("0x{}", hex::encode(encoded))
    }

    fn erc20_info(
        from: Address,
        token: Address,
        spender: Address,
        signed_transaction: String,
    ) -> Erc20ApprovalGasSponsoringInfo {
        Erc20ApprovalGasSponsoringInfo {
            from,
            asset: token,
            spender,
            amount: "1000000".into(),
            signed_transaction,
            version: "1".into(),
        }
    }

    #[test]
    fn erc20_approval_payload_stub_does_not_cover_without_signed_tx() {
        let payer = Address::repeat_byte(0x11);
        let token = Address::repeat_byte(0xAA);
        let info = erc20_info(payer, token, PERMIT2_ADDRESS, "0xdead".into());
        let err = validate_erc20_approval_for_payment(&info, payer, token).unwrap_err();
        assert_eq!(
            err.as_payment_problem().reason().as_str(),
            "erc20_approval_tx_parse_failed"
        );
    }

    #[test]
    fn erc20_approval_valid_signed_tx_covers() {
        use std::str::FromStr;

        use alloy_signer_local::PrivateKeySigner;
        let signer = PrivateKeySigner::from_str(
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        )
        .unwrap();
        let payer = signer.address();
        let token = Address::repeat_byte(0xAA);
        let signed = sign_approve_tx(&signer, token, PERMIT2_ADDRESS);
        let info = erc20_info(payer, token, PERMIT2_ADDRESS, signed);
        validate_erc20_approval_for_payment(&info, payer, token).expect("valid approve tx");
    }

    #[test]
    fn erc20_approval_wrong_target_is_rejected() {
        use std::str::FromStr;

        use alloy_signer_local::PrivateKeySigner;
        let signer = PrivateKeySigner::from_str(
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        )
        .unwrap();
        let payer = signer.address();
        let token = Address::repeat_byte(0xAA);
        let other = Address::repeat_byte(0xBB);
        let signed = sign_approve_tx(&signer, other, PERMIT2_ADDRESS);
        let info = erc20_info(payer, token, PERMIT2_ADDRESS, signed);
        let err = validate_erc20_approval_for_payment(&info, payer, token).unwrap_err();
        assert_eq!(
            err.as_payment_problem().reason().as_str(),
            "erc20_approval_tx_wrong_target"
        );
    }

    #[test]
    fn permit2_extension_ignores_erc20_when_facilitator_has_no_extension() {
        let payer = Address::repeat_byte(0x11);
        let token = Address::repeat_byte(0xAA);
        let mut extensions = Extensions::new();
        extensions.insert(
            ERC20_APPROVAL_GAS_SPONSORING_KEY,
            ExtensionEntry::info(serde_json::json!({
                "from": format!("{payer:#x}"),
                "asset": format!("{token:#x}"),
                "spender": format!("{PERMIT2_ADDRESS:#x}"),
                "amount": "1000000",
                "signedTransaction": "0xdead",
                "version": "1",
            })),
        );
        let covers = permit2_extension_covers_allowance(&extensions, payer, token, 6, false)
            .expect("payload-only stub is ignored");
        assert!(!covers);
    }
}
