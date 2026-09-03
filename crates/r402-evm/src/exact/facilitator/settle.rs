//! On-chain settlement logic for the EIP-155 exact scheme.
//!
//! Contains the [`settle_payment`] function and the prepared
//! `transferWithAuthorization` call wrapper types.

use alloy_contract::SolCallBuilder;
use alloy_primitives::{Address, B256, Bytes, Signature, TxHash, U256};
use alloy_provider::bindings::IMulticall3;
use alloy_provider::{MULTICALL3_ADDRESS, MulticallItem, Provider};
use alloy_sol_types::{Eip712Domain, SolCall, SolEvent};
use alloy_transport::TransportError;
use r402_protocol::error::VerificationError;
#[cfg(feature = "telemetry")]
use tracing_core::Level;

use super::contract::{IEIP3009, IX402Permit2Proxy};
use super::eip6492::{FACTORY_NOT_ALLOWED, deny_undeployed_factory};
use super::signature::SignedMessage;
use super::{Eip3009Payment, Permit2Payment};
use crate::chain::contracts::IERC20;
use crate::chain::{Eip155MetaTransactionProvider, MetaTransaction};
use crate::eip2612::Eip2612SignedPermit;
use crate::error::Eip155ExactError;
use crate::exact::X402_EXACT_PERMIT2_PROXY;
use crate::signature::{ClassifiedSignature, classify_with_code};

/// Prepared `transferWithAuthorization` call using a raw bytes signature.
pub(super) struct TransferWithAuthorization0Call<P>(
    pub TransferWithAuthorizationCall<P, IEIP3009::transferWithAuthorization_0Call, Bytes>,
);

impl<P> std::fmt::Debug for TransferWithAuthorization0Call<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransferWithAuthorization0Call")
            .finish_non_exhaustive()
    }
}

impl<'a, P: Provider> TransferWithAuthorization0Call<&'a P> {
    /// Constructs a full `transferWithAuthorization` call for a verified payment payload.
    pub(super) fn new(
        contract: &'a IEIP3009::IEIP3009Instance<P>,
        payment: &Eip3009Payment,
        signature: Bytes,
    ) -> Self {
        let from = payment.from;
        let to = payment.to;
        let value = payment.value;
        let valid_after = U256::from(payment.valid_after.as_secs());
        let valid_before = U256::from(payment.valid_before.as_secs());
        let nonce = payment.nonce;
        let tx = contract.transferWithAuthorization_0(
            from,
            to,
            value,
            valid_after,
            valid_before,
            nonce,
            signature.clone(),
        );
        TransferWithAuthorization0Call(TransferWithAuthorizationCall {
            tx,
            from,
            to,
            value,
            valid_after,
            valid_before,
            nonce,
            signature,
            contract_address: *contract.address(),
        })
    }
}

/// Prepared `transferWithAuthorization` call using split (v, r, s) signature.
pub(super) struct TransferWithAuthorization1Call<P>(
    pub TransferWithAuthorizationCall<P, IEIP3009::transferWithAuthorization_1Call, Signature>,
);

impl<P> std::fmt::Debug for TransferWithAuthorization1Call<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransferWithAuthorization1Call")
            .finish_non_exhaustive()
    }
}

impl<'a, P: Provider> TransferWithAuthorization1Call<&'a P> {
    /// Constructs a full `transferWithAuthorization` call for a verified payment payload
    /// using split signature components (v, r, s).
    pub(super) fn new(
        contract: &'a IEIP3009::IEIP3009Instance<P>,
        payment: &Eip3009Payment,
        signature: Signature,
    ) -> Self {
        let from = payment.from;
        let to = payment.to;
        let value = payment.value;
        let valid_after = U256::from(payment.valid_after.as_secs());
        let valid_before = U256::from(payment.valid_before.as_secs());
        let nonce = payment.nonce;
        let v = 27 + u8::from(signature.v());
        let r = B256::from(signature.r());
        let s = B256::from(signature.s());
        let tx = contract.transferWithAuthorization_1(
            from,
            to,
            value,
            valid_after,
            valid_before,
            nonce,
            v,
            r,
            s,
        );
        TransferWithAuthorization1Call(TransferWithAuthorizationCall {
            tx,
            from,
            to,
            value,
            valid_after,
            valid_before,
            nonce,
            signature,
            contract_address: *contract.address(),
        })
    }
}

/// A prepared call to `transferWithAuthorization` (ERC-3009) including all derived fields.
#[allow(
    missing_debug_implementations,
    reason = "generic type params may not impl Debug"
)]
#[allow(
    dead_code,
    reason = "fields are read by transfer_span! when telemetry is on"
)]
pub(super) struct TransferWithAuthorizationCall<P, TCall, TSignature> {
    /// The prepared call builder that can be `.call()`ed or `.send()`ed.
    pub tx: SolCallBuilder<P, TCall>,
    /// The sender (`from`) address for the authorization.
    pub from: Address,
    /// The recipient (`to`) address for the authorization.
    pub to: Address,
    /// The amount to transfer (value).
    pub value: U256,
    /// Start of the validity window (inclusive).
    pub valid_after: U256,
    /// End of the validity window (exclusive).
    pub valid_before: U256,
    /// 32-byte authorization nonce (prevents replay).
    pub nonce: B256,
    /// EIP-712 signature for the transfer authorization.
    pub signature: TSignature,
    /// Address of the token contract used for this transfer.
    pub contract_address: Address,
}

/// Settles a verified payment by sending the transfer transaction on-chain.
///
/// # Errors
///
/// Returns [`Eip155ExactError`] if the on-chain settlement transaction fails.
///
/// # Panics
///
/// Panics if the authorization deadline timestamp overflows `i64`.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_lines,
    reason = "settlement logic is inherently complex"
)]
pub(super) async fn settle_payment<P, E>(
    provider: &P,
    contract: &IEIP3009::IEIP3009Instance<&P::Inner>,
    payment: &Eip3009Payment,
    eip712_domain: &Eip712Domain,
    eip6492_allowed_factories: &[Address],
    data_suffix: &[u8],
) -> Result<TxHash, Eip155ExactError>
where
    P: Eip155MetaTransactionProvider<Error = E> + Sync,
    Eip155ExactError: From<E>,
{
    let signed_message = SignedMessage::extract(payment, eip712_domain)?;
    let payer = payment.from;
    let classified = classify_with_code(
        provider.inner(),
        payer,
        signed_message.signature,
        &signed_message.hash,
    )
    .await?;
    let expected = ExpectedTransfer {
        token: *contract.address(),
        from: payment.from,
        to: payment.to,
        value: payment.value,
    };
    let receipt = match classified {
        ClassifiedSignature::EIP6492 {
            factory,
            factory_calldata,
            inner,
            original: _,
        } => {
            let is_deployed = is_contract_deployed(provider.inner(), &payer).await?;
            if !is_deployed && deny_undeployed_factory(factory, &[], eip6492_allowed_factories) {
                #[cfg(feature = "telemetry")]
                tracing::warn!(factory = %factory, "eip6492_factory_not_allowed");
                return Err(VerificationError::from_wire(FACTORY_NOT_ALLOWED).into());
            }
            let transfer_call = TransferWithAuthorization0Call::new(contract, payment, inner);
            let transfer_call = transfer_call.0;
            if is_deployed {
                let tx_fut = Eip155MetaTransactionProvider::send_transaction(
                    provider,
                    MetaTransaction {
                        to: transfer_call.tx.target(),
                        calldata: transfer_call.tx.calldata().clone(),
                        confirmations: 1,
                        from: None,
                    }
                    .with_data_suffix(data_suffix),
                );
                traced!(
                    tx_fut,
                    transfer_span!(
                        "call_transferWithAuthorization_0",
                        transfer_call,
                        sig_kind = "EIP6492.deployed"
                    )
                )?
            } else {
                let deployment_call = IMulticall3::Call3 {
                    allowFailure: true,
                    target: factory,
                    callData: factory_calldata,
                };
                let transfer_with_authorization_call = IMulticall3::Call3 {
                    allowFailure: false,
                    target: transfer_call.tx.target(),
                    callData: transfer_call.tx.calldata().clone(),
                };
                let aggregate_call = IMulticall3::aggregate3Call {
                    calls: vec![deployment_call, transfer_with_authorization_call],
                };
                let tx_fut = Eip155MetaTransactionProvider::send_transaction(
                    provider,
                    MetaTransaction {
                        to: MULTICALL3_ADDRESS,
                        calldata: aggregate_call.abi_encode().into(),
                        confirmations: 1,
                        from: None,
                    }
                    .with_data_suffix(data_suffix),
                );
                traced!(
                    tx_fut,
                    transfer_span!(
                        "call_transferWithAuthorization_0",
                        transfer_call,
                        sig_kind = "EIP6492.counterfactual"
                    )
                )?
            }
        }
        ClassifiedSignature::EIP1271(eip1271_signature) => {
            let transfer_call =
                TransferWithAuthorization0Call::new(contract, payment, eip1271_signature);
            let transfer_call = transfer_call.0;
            let tx_fut = Eip155MetaTransactionProvider::send_transaction(
                provider,
                MetaTransaction {
                    to: transfer_call.tx.target(),
                    calldata: transfer_call.tx.calldata().clone(),
                    confirmations: 1,
                    from: None,
                }
                .with_data_suffix(data_suffix),
            );
            traced!(
                tx_fut,
                transfer_span!(
                    "call_transferWithAuthorization_0",
                    transfer_call,
                    sig_kind = "EIP1271"
                )
            )?
        }
        ClassifiedSignature::Eoa(signature) => {
            let transfer_call = TransferWithAuthorization1Call::new(contract, payment, signature);
            let transfer_call = transfer_call.0;
            let tx_fut = Eip155MetaTransactionProvider::send_transaction(
                provider,
                MetaTransaction {
                    to: transfer_call.tx.target(),
                    calldata: transfer_call.tx.calldata().clone(),
                    confirmations: 1,
                    from: None,
                }
                .with_data_suffix(data_suffix),
            );
            traced!(
                tx_fut,
                transfer_span!(
                    "call_transferWithAuthorization_1",
                    transfer_call,
                    sig_kind = "EOA"
                )
            )?
        }
    };
    check_receipt(&receipt, "transferWithAuthorization", Some(expected))
}

/// Lowers a wire [`Eip2612SignedPermit`] into the ABI struct for `settleWithPermit`.
pub(super) fn build_eip2612_abi(
    eip2612: &Eip2612SignedPermit,
) -> Result<IX402Permit2Proxy::EIP2612Permit, VerificationError> {
    let (r, s, v) = eip2612.split_signature().ok_or_else(|| {
        VerificationError::InvalidFormat(
            "eip2612GasSponsoring signature is not the canonical 65-byte (r,s,v) form".into(),
        )
    })?;
    Ok(IX402Permit2Proxy::EIP2612Permit {
        value: eip2612.amount.into(),
        deadline: eip2612.deadline.into(),
        r,
        s,
        v,
    })
}

/// Settles a verified Permit2 payment via `settle` or `settleWithPermit`.
///
/// When `eip2612GasSponsoring` is attached, the proxy atomically calls
/// `token.permit` then Permit2 transfer.
///
/// # Errors
///
/// Returns [`Eip155ExactError`] if the on-chain settlement transaction fails.
#[allow(
    clippy::cognitive_complexity,
    reason = "settlement logic is inherently complex"
)]
pub(super) async fn settle_permit2_payment<P, E>(
    provider: &P,
    payment: &Permit2Payment,
    data_suffix: &[u8],
) -> Result<TxHash, Eip155ExactError>
where
    P: Eip155MetaTransactionProvider<Error = E> + Sync,
    Eip155ExactError: From<E>,
{
    let proxy = IX402Permit2Proxy::new(X402_EXACT_PERMIT2_PROXY, provider.inner());

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

    let calldata = match payment.eip2612.as_ref() {
        Some(eip2612) => proxy
            .settleWithPermit(
                build_eip2612_abi(eip2612)?,
                permit,
                payment.from,
                witness,
                payment.signature.clone(),
            )
            .calldata()
            .clone(),
        None => proxy
            .settle(permit, payment.from, witness, payment.signature.clone())
            .calldata()
            .clone(),
    };

    let tx_fut = Eip155MetaTransactionProvider::send_transaction(
        provider,
        MetaTransaction {
            to: X402_EXACT_PERMIT2_PROXY,
            calldata,
            confirmations: 1,
            from: None,
        }
        .with_data_suffix(data_suffix),
    );
    let receipt = traced!(
        tx_fut,
        tracing::info_span!("settle_permit2",
            from = %payment.from,
            to = %payment.to,
            token = %payment.token,
            amount = %payment.amount,
            otel.kind = "client",
        )
    )?;

    let expected = ExpectedTransfer {
        token: payment.token,
        from: payment.from,
        to: payment.to,
        value: payment.amount,
    };
    check_receipt(&receipt, "Permit2 settle", Some(expected))
}

/// Expected ERC-20 Transfer to match against a success receipt.
#[derive(Debug, Clone, Copy)]
pub(super) struct ExpectedTransfer {
    pub token: Address,
    pub from: Address,
    pub to: Address,
    pub value: U256,
}

/// Checks the transaction receipt status and, when logs are present, the Transfer event.
#[allow(clippy::missing_const_for_fn, reason = "telemetry branch is not const")]
fn check_receipt(
    receipt: &alloy_rpc_types_eth::TransactionReceipt,
    #[cfg_attr(
        not(feature = "telemetry"),
        allow(unused_variables, reason = "label is telemetry-only")
    )]
    label: &str,
    expected: Option<ExpectedTransfer>,
) -> Result<TxHash, Eip155ExactError> {
    if !receipt.status() {
        return receipt_reverted(receipt, label);
    }
    if let Some(expected) = expected {
        check_receipt_transfer(receipt, label, expected)?;
    }
    log_receipt_ok(receipt, label);
    Ok(receipt.transaction_hash)
}

fn receipt_reverted(
    receipt: &alloy_rpc_types_eth::TransactionReceipt,
    #[cfg_attr(
        not(feature = "telemetry"),
        allow(unused_variables, reason = "label is telemetry-only")
    )]
    label: &str,
) -> Result<TxHash, Eip155ExactError> {
    #[cfg(feature = "telemetry")]
    tracing::event!(Level::WARN, status = "failed", tx = %receipt.transaction_hash, "{label} failed");
    Err(Eip155ExactError::TransactionReverted(
        receipt.transaction_hash,
    ))
}

fn check_receipt_transfer(
    receipt: &alloy_rpc_types_eth::TransactionReceipt,
    #[cfg_attr(
        not(feature = "telemetry"),
        allow(unused_variables, reason = "label is telemetry-only")
    )]
    label: &str,
    expected: ExpectedTransfer,
) -> Result<(), Eip155ExactError> {
    if transfer_event_matches(receipt.logs(), expected) {
        return Ok(());
    }
    #[cfg(feature = "telemetry")]
    tracing::event!(Level::WARN, status = "mismatch", tx = %receipt.transaction_hash, "{label} transfer event mismatch");
    Err(Eip155ExactError::TransferEventMismatch(
        receipt.transaction_hash,
    ))
}

fn log_receipt_ok(
    #[cfg_attr(
        not(feature = "telemetry"),
        allow(unused_variables, reason = "receipt hash is telemetry-only")
    )]
    receipt: &alloy_rpc_types_eth::TransactionReceipt,
    #[cfg_attr(
        not(feature = "telemetry"),
        allow(unused_variables, reason = "label is telemetry-only")
    )]
    label: &str,
) {
    #[cfg(feature = "telemetry")]
    tracing::event!(Level::INFO, status = "ok", tx = %receipt.transaction_hash, "{label} succeeded");
}

fn transfer_event_matches(logs: &[alloy_rpc_types_eth::Log], expected: ExpectedTransfer) -> bool {
    logs.iter().any(|log| {
        if log.address() != expected.token {
            return false;
        }
        IERC20::Transfer::decode_log(&log.inner).is_ok_and(|ev| {
            ev.from == expected.from && ev.to == expected.to && ev.value == expected.value
        })
    })
}

pub(super) async fn reconcile_pending_receipt<P: Provider>(
    provider: &P,
    hash: TxHash,
    expected: ExpectedTransfer,
) -> Result<TxHash, Eip155ExactError> {
    match provider.get_transaction_receipt(hash).await {
        Ok(Some(receipt)) => check_receipt(&receipt, "pending reconcile", Some(expected)),
        Ok(None) => Err(Eip155ExactError::ReceiptWait {
            hash,
            source: alloy_provider::PendingTransactionError::FailedToRegister,
        }),
        Err(e) => Err(Eip155ExactError::ReceiptWait {
            hash,
            source: e.into(),
        }),
    }
}

/// Check whether contract code is present at `address`.
async fn is_contract_deployed<P: Provider>(
    provider: &P,
    address: &Address,
) -> Result<bool, TransportError> {
    let bytes_fut = provider.get_code_at(*address).into_future();
    let bytes = traced!(
        bytes_fut,
        tracing::info_span!("get_code_at",
            address = %address,
            otel.kind = "client",
        )
    )?;
    Ok(!bytes.is_empty())
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{B256, Log as PrimLog, LogData};

    use super::*;

    fn transfer_log(
        token: Address,
        from: Address,
        to: Address,
        value: U256,
    ) -> alloy_rpc_types_eth::Log {
        let topics = vec![
            IERC20::Transfer::SIGNATURE_HASH,
            B256::from(from.into_word()),
            B256::from(to.into_word()),
        ];
        let inner = PrimLog {
            address: token,
            data: LogData::new_unchecked(topics, value.to_be_bytes::<32>().into()),
        };
        alloy_rpc_types_eth::Log {
            inner,
            block_hash: None,
            block_number: None,
            block_timestamp: None,
            transaction_hash: None,
            transaction_index: None,
            log_index: None,
            removed: false,
        }
    }

    #[test]
    fn empty_logs_fail_transfer_match() {
        let expected = ExpectedTransfer {
            token: Address::repeat_byte(0xAA),
            from: Address::repeat_byte(0x11),
            to: Address::repeat_byte(0x22),
            value: U256::from(1_000_000_u64),
        };
        assert!(!transfer_event_matches(&[], expected));
    }

    #[test]
    fn matching_transfer_log_passes() {
        let token = Address::repeat_byte(0xAA);
        let from = Address::repeat_byte(0x11);
        let to = Address::repeat_byte(0x22);
        let value = U256::from(1_000_000_u64);
        let logs = vec![transfer_log(token, from, to, value)];
        let expected = ExpectedTransfer {
            token,
            from,
            to,
            value,
        };
        assert!(transfer_event_matches(&logs, expected));
    }

    #[test]
    fn settle_with_permit_selector_differs_from_settle() {
        use alloy_sol_types::SolCall;
        assert_ne!(
            IX402Permit2Proxy::settleCall::SELECTOR,
            IX402Permit2Proxy::settleWithPermitCall::SELECTOR
        );
    }

    #[test]
    fn wrong_value_fails_transfer_match() {
        let token = Address::repeat_byte(0xAA);
        let from = Address::repeat_byte(0x11);
        let to = Address::repeat_byte(0x22);
        let logs = vec![transfer_log(token, from, to, U256::from(1_u64))];
        let expected = ExpectedTransfer {
            token,
            from,
            to,
            value: U256::from(1_000_000_u64),
        };
        assert!(!transfer_event_matches(&logs, expected));
    }
}
