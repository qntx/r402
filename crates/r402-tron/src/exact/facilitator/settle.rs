//! On-chain settlement logic for the Tron exact scheme.
//!
//! Builds, signs, broadcasts, and confirms `transferWithAuthorization` and
//! `x402ExactPermit2Proxy.settle` transactions via the `TronGrid` HTTP API.

use alloy_primitives::{Bytes, U256};
use alloy_sol_types::SolCall;

use super::signature::SignedMessage;
use super::{Eip3009Payment, Permit2Payment};
use crate::chain::TronChainReference;
use crate::chain::contracts::{eip3009, x402_exact_permit2_proxy};
use crate::exact::TronExactError;

/// Settles a verified EIP-3009 payment by calling `transferWithAuthorization`
/// on the token contract via `TronGrid`.
pub(super) async fn settle_payment(
    provider: &crate::chain::TronChainProvider,
    payment: &Eip3009Payment,
    eip712_domain: &alloy_sol_types::Eip712Domain,
) -> Result<String, TronExactError> {
    let signed_message = SignedMessage::extract(payment, eip712_domain).map_err(|e| match e {
        super::signature::TronSignatureError::SignerMismatch => TronExactError::SignerMismatch,
        other => TronExactError::SignatureRecovery(other.to_string()),
    })?;
    let _ = &signed_message;

    // Build the calldata for transferWithAuthorization (bytes signature variant)
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

    let tx_id = provider
        .send_contract_call(token, Bytes::from(calldata))
        .await?;
    Ok(tx_id)
}

/// Settles a verified Permit2 payment by calling `x402ExactPermit2Proxy.settle`
/// via `TronGrid`.
pub(super) async fn settle_permit2_payment(
    provider: &crate::chain::TronChainProvider,
    chain: &TronChainReference,
    payment: &Permit2Payment,
) -> Result<String, TronExactError> {
    let proxy_addr = chain
        .x402_exact_permit2_proxy()
        .ok_or(TronExactError::UnsupportedTransferMethod)?;

    let call = x402_exact_permit2_proxy::settleCall {
        permit: x402_exact_permit2_proxy::TronPermitTransferFrom {
            permitted: x402_exact_permit2_proxy::TronTokenPermissions {
                token: payment.token,
                amount: payment.amount,
            },
            nonce: payment.nonce,
            deadline: payment.deadline,
        },
        owner: payment.from,
        witness: x402_exact_permit2_proxy::TronWitness {
            to: payment.to,
            validAfter: payment.valid_after,
        },
        signature: payment.signature.clone(),
    };
    let calldata = x402_exact_permit2_proxy::settleCall::abi_encode(&call);

    let tx_id = provider
        .send_contract_call(proxy_addr.as_evm(), Bytes::from(calldata))
        .await?;
    Ok(tx_id)
}
