//! On-chain settlement logic for the EIP-155 exact scheme.
//!
//! Contains the [`settle_payment`] function and the prepared
//! `transferWithAuthorization` call wrapper types.

use alloy_contract::SolCallBuilder;
use alloy_primitives::{Address, B256, Bytes, Signature, TxHash, U256};
use alloy_provider::bindings::IMulticall3;
use alloy_provider::{MULTICALL3_ADDRESS, MulticallItem, Provider};
use alloy_sol_types::{Eip712Domain, SolCall};
use alloy_transport::TransportError;
#[cfg(feature = "telemetry")]
use tracing_core::Level;

use super::Eip3009Payment;
use super::Permit2Payment;
use super::contract::{IEIP3009, IX402Permit2Proxy};
use super::error::Eip155ExactError;
use super::signature::{SignedMessage, StructuredSignature};
use crate::chain::{Eip155MetaTransactionProvider, MetaTransaction};
use crate::exact::X402_EXACT_PERMIT2_PROXY;

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

/// Prepared `transferWithAuthorization` call using a raw bytes signature.
pub struct TransferWithAuthorization0Call<P>(
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
    pub fn new(
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
pub struct TransferWithAuthorization1Call<P>(
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
    pub fn new(
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
#[allow(missing_debug_implementations)]
pub struct TransferWithAuthorizationCall<P, TCall, TSignature> {
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

/// Settles a verified payment by sending the transfer transaction on-chain.
///
/// # Errors
///
/// Returns [`Eip155ExactError`] if the on-chain settlement transaction fails.
///
/// # Panics
///
/// Panics if the authorization deadline timestamp overflows `i64`.
#[allow(clippy::cognitive_complexity)]
pub async fn settle_payment<P, E>(
    provider: &P,
    contract: &IEIP3009::IEIP3009Instance<&P::Inner>,
    payment: &Eip3009Payment,
    eip712_domain: &Eip712Domain,
) -> Result<TxHash, Eip155ExactError>
where
    P: Eip155MetaTransactionProvider<Error = E> + Sync,
    Eip155ExactError: From<E>,
{
    let signed_message = SignedMessage::extract(payment, eip712_domain)?;
    let payer = payment.from;
    let receipt = match signed_message.signature {
        StructuredSignature::EIP6492 {
            factory,
            factory_calldata,
            inner,
            original: _,
        } => {
            let is_deployed = is_contract_deployed(provider.inner(), &payer).await?;
            let transfer_call = TransferWithAuthorization0Call::new(contract, payment, inner);
            let transfer_call = transfer_call.0;
            if is_deployed {
                let tx_fut = Eip155MetaTransactionProvider::send_transaction(
                    provider,
                    MetaTransaction {
                        to: transfer_call.tx.target(),
                        calldata: transfer_call.tx.calldata().clone(),
                        confirmations: 1,
                    },
                );
                traced!(
                    tx_fut,
                    tracing::info_span!("call_transferWithAuthorization_0",
                        from = %transfer_call.from,
                        to = %transfer_call.to,
                        value = %transfer_call.value,
                        valid_after = %transfer_call.valid_after,
                        valid_before = %transfer_call.valid_before,
                        nonce = %transfer_call.nonce,
                        signature = %transfer_call.signature,
                        token_contract = %transfer_call.contract_address,
                        sig_kind="EIP6492.deployed",
                        otel.kind = "client",
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
                    },
                );
                traced!(
                    tx_fut,
                    tracing::info_span!("call_transferWithAuthorization_0",
                        from = %transfer_call.from,
                        to = %transfer_call.to,
                        value = %transfer_call.value,
                        valid_after = %transfer_call.valid_after,
                        valid_before = %transfer_call.valid_before,
                        nonce = %transfer_call.nonce,
                        signature = %transfer_call.signature,
                        token_contract = %transfer_call.contract_address,
                        sig_kind="EIP6492.counterfactual",
                        otel.kind = "client",
                    )
                )?
            }
        }
        StructuredSignature::EIP1271(eip1271_signature) => {
            let transfer_call =
                TransferWithAuthorization0Call::new(contract, payment, eip1271_signature);
            let transfer_call = transfer_call.0;
            let tx_fut = Eip155MetaTransactionProvider::send_transaction(
                provider,
                MetaTransaction {
                    to: transfer_call.tx.target(),
                    calldata: transfer_call.tx.calldata().clone(),
                    confirmations: 1,
                },
            );
            traced!(
                tx_fut,
                tracing::info_span!("call_transferWithAuthorization_0",
                    from = %transfer_call.from,
                    to = %transfer_call.to,
                    value = %transfer_call.value,
                    valid_after = %transfer_call.valid_after,
                    valid_before = %transfer_call.valid_before,
                    nonce = %transfer_call.nonce,
                    signature = %transfer_call.signature,
                    token_contract = %transfer_call.contract_address,
                    sig_kind="EIP1271",
                    otel.kind = "client",
                )
            )?
        }
        StructuredSignature::EOA(signature) => {
            let transfer_call = TransferWithAuthorization1Call::new(contract, payment, signature);
            let transfer_call = transfer_call.0;
            let tx_fut = Eip155MetaTransactionProvider::send_transaction(
                provider,
                MetaTransaction {
                    to: transfer_call.tx.target(),
                    calldata: transfer_call.tx.calldata().clone(),
                    confirmations: 1,
                },
            );
            traced!(
                tx_fut,
                tracing::info_span!("call_transferWithAuthorization_1",
                    from = %transfer_call.from,
                    to = %transfer_call.to,
                    value = %transfer_call.value,
                    valid_after = %transfer_call.valid_after,
                    valid_before = %transfer_call.valid_before,
                    nonce = %transfer_call.nonce,
                    signature = %transfer_call.signature,
                    token_contract = %transfer_call.contract_address,
                    sig_kind="EOA",
                    otel.kind = "client",
                )
            )?
        }
    };
    let success = receipt.status();
    if success {
        #[cfg(feature = "telemetry")]
        tracing::event!(Level::INFO,
            status = "ok",
            tx = %receipt.transaction_hash,
            "transferWithAuthorization succeeded"
        );
        Ok(receipt.transaction_hash)
    } else {
        #[cfg(feature = "telemetry")]
        tracing::event!(
            Level::WARN,
            status = "failed",
            tx = %receipt.transaction_hash,
            "transferWithAuthorization failed"
        );
        Err(Eip155ExactError::TransactionReverted(
            receipt.transaction_hash,
        ))
    }
}

/// Settles a verified Permit2 payment by calling `x402ExactPermit2Proxy.settle()`.
///
/// # Errors
///
/// Returns [`Eip155ExactError`] if the on-chain settlement transaction fails.
#[allow(clippy::cognitive_complexity)]
pub async fn settle_permit2_payment<P, E>(
    provider: &P,
    payment: &Permit2Payment,
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
        extra: payment.extra.clone(),
    };

    let settle_call = proxy.settle(permit, payment.from, witness, payment.signature.clone());
    let calldata = settle_call.calldata().clone();

    let tx_fut = Eip155MetaTransactionProvider::send_transaction(
        provider,
        MetaTransaction {
            to: X402_EXACT_PERMIT2_PROXY,
            calldata,
            confirmations: 1,
        },
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

    let success = receipt.status();
    if success {
        #[cfg(feature = "telemetry")]
        tracing::event!(Level::INFO,
            status = "ok",
            tx = %receipt.transaction_hash,
            "Permit2 settle succeeded"
        );
        Ok(receipt.transaction_hash)
    } else {
        #[cfg(feature = "telemetry")]
        tracing::event!(
            Level::WARN,
            status = "failed",
            tx = %receipt.transaction_hash,
            "Permit2 settle failed"
        );
        Err(Eip155ExactError::TransactionReverted(
            receipt.transaction_hash,
        ))
    }
}
