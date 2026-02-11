//! Facilitator-side payment verification and settlement for EIP-155 exact scheme.
//!
//! This module implements the facilitator logic for verifying and settling
//! EVM exact payments on EVM chains. It currently supports EIP-3009
//! (`transferWithAuthorization`) and routes based on [`ExactPayload`] variants.
//!
//! Key capabilities:
//! - Signature verification (EOA, EIP-1271, EIP-6492)
//! - Balance and amount validation
//! - EIP-712 domain construction
//! - On-chain settlement with gas management
//! - Smart wallet deployment for counterfactual signatures

mod contract;
mod error;
mod settle;
mod signature;
mod verify;

pub use contract::{IEIP3009, IX402Permit2Proxy, Validator6492};
pub use error::Eip155ExactError;
pub use settle::{
    TransferWithAuthorization0Call, TransferWithAuthorization1Call, TransferWithAuthorizationCall,
    settle_payment, settle_permit2_payment,
};
pub use signature::StructuredSignatureFormatError;
pub use verify::{
    assert_domain, assert_enough_balance, assert_enough_value, assert_time, verify_payment,
    verify_permit2_payment,
};

use alloy_primitives::{Address, B256, Bytes, U256, address};
use alloy_provider::Provider;
use r402::chain::ChainProvider;
use r402::facilitator::{Facilitator, FacilitatorError};
use r402::proto;
use r402::proto::UnixTimestamp;
use r402::proto::{v1, v2};
use r402::scheme::{SchemeBuilder, SchemeId};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use crate::chain::Eip155MetaTransactionProvider;
use crate::exact::types;
use crate::exact::{ExactPayload, ExactScheme, V1Eip155Exact, V2Eip155Exact};

/// Signature verifier for EIP-6492, EIP-1271, EOA, universally deployed on the supported EVM chains.
/// If absent on a target chain, verification will fail; you should deploy the validator there.
pub const VALIDATOR_ADDRESS: Address = address!("0xdAcD51A54883eb67D95FAEb2BBfdC4a9a6BD2a3B");

/// A fully specified ERC-3009 authorization payload for EVM settlement.
#[derive(Debug)]
pub struct Eip3009Payment {
    /// Authorized sender (`from`) — EOA or smart wallet.
    pub from: Address,
    /// Authorized recipient (`to`).
    pub to: Address,
    /// Transfer amount (token units).
    pub value: U256,
    /// Not valid before this timestamp (inclusive).
    pub valid_after: UnixTimestamp,
    /// Not valid at/after this timestamp (exclusive).
    pub valid_before: UnixTimestamp,
    /// Unique 32-byte nonce (prevents replay).
    pub nonce: B256,
    /// Raw signature bytes (EIP-1271 or EIP-6492-wrapped).
    pub signature: Bytes,
}

/// A fully specified Permit2 authorization payload for EVM settlement.
#[derive(Debug)]
pub struct Permit2Payment {
    /// Signer / owner address.
    pub from: Address,
    /// Destination address for funds.
    pub to: Address,
    /// Token contract address.
    pub token: Address,
    /// Permitted amount (token units).
    pub amount: U256,
    /// Must be the `x402Permit2Proxy` address.
    pub spender: Address,
    /// Unique nonce (uint256).
    pub nonce: U256,
    /// Signature expires after this unix timestamp.
    pub deadline: U256,
    /// Payment invalid before this timestamp.
    pub valid_after: U256,
    /// Extra witness data (typically empty `0x`).
    pub extra: Bytes,
    /// EIP-712 signature bytes.
    pub signature: Bytes,
}

impl<P> SchemeBuilder<P> for V1Eip155Exact
where
    P: Eip155MetaTransactionProvider + ChainProvider + Send + Sync + 'static,
    Eip155ExactError: From<P::Error>,
{
    fn build(
        &self,
        provider: P,
        _config: Option<serde_json::Value>,
    ) -> Result<Box<dyn Facilitator>, Box<dyn std::error::Error>> {
        Ok(Box::new(V1Eip155ExactFacilitator::new(provider)))
    }
}

impl<P> SchemeBuilder<P> for V2Eip155Exact
where
    P: Eip155MetaTransactionProvider + ChainProvider + Send + Sync + 'static,
    Eip155ExactError: From<P::Error>,
{
    fn build(
        &self,
        provider: P,
        _config: Option<serde_json::Value>,
    ) -> Result<Box<dyn Facilitator>, Box<dyn std::error::Error>> {
        Ok(Box::new(V2Eip155ExactFacilitator::new(provider)))
    }
}

/// Facilitator for V1 EIP-155 exact scheme payments.
///
/// V1 only supports EIP-3009 (`transferWithAuthorization`). Permit2 payloads
/// are rejected with [`PaymentVerificationError::UnsupportedScheme`].
pub struct V1Eip155ExactFacilitator<P> {
    provider: P,
}

impl<P> std::fmt::Debug for V1Eip155ExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V1Eip155ExactFacilitator")
            .finish_non_exhaustive()
    }
}

impl<P> V1Eip155ExactFacilitator<P> {
    /// Creates a new V1 EIP-155 exact scheme facilitator with the given provider.
    pub const fn new(provider: P) -> Self {
        Self { provider }
    }
}

impl<P> Facilitator for V1Eip155ExactFacilitator<P>
where
    P: Eip155MetaTransactionProvider + ChainProvider + Send + Sync,
    P::Inner: Provider,
    Eip155ExactError: From<P::Error>,
{
    fn verify(
        &self,
        request: proto::VerifyRequest,
    ) -> Pin<Box<dyn Future<Output = Result<proto::VerifyResponse, FacilitatorError>> + Send + '_>>
    {
        Box::pin(async move {
            let request = types::v1::VerifyRequest::from_proto(request)?;
            let payload = &request.payment_payload;
            let requirements = &request.payment_requirements;
            let eip3009 = match &payload.payload {
                ExactPayload::Eip3009(p) => p,
                ExactPayload::Permit2(_) => {
                    return Err(FacilitatorError::PaymentVerification(
                        proto::PaymentVerificationError::UnsupportedScheme,
                    ));
                }
            };
            let (contract, payment, eip712_domain) = verify::assert_valid_v1_payment(
                self.provider.inner(),
                self.provider.chain(),
                eip3009,
                payload,
                requirements,
            )
            .await?;

            let payer =
                verify_payment(self.provider.inner(), &contract, &payment, &eip712_domain).await?;

            Ok(v1::VerifyResponse::valid(payer.to_string()))
        })
    }

    fn settle(
        &self,
        request: proto::SettleRequest,
    ) -> Pin<Box<dyn Future<Output = Result<proto::SettleResponse, FacilitatorError>> + Send + '_>>
    {
        Box::pin(async move {
            let request = types::v1::SettleRequest::from_settle(request)?;
            let payload = &request.payment_payload;
            let requirements = &request.payment_requirements;
            let eip3009 = match &payload.payload {
                ExactPayload::Eip3009(p) => p,
                ExactPayload::Permit2(_) => {
                    return Err(FacilitatorError::PaymentVerification(
                        proto::PaymentVerificationError::UnsupportedScheme,
                    ));
                }
            };
            let (contract, payment, eip712_domain) = verify::assert_valid_v1_payment(
                self.provider.inner(),
                self.provider.chain(),
                eip3009,
                payload,
                requirements,
            )
            .await?;

            let tx_hash =
                settle_payment(&self.provider, &contract, &payment, &eip712_domain).await?;
            Ok(v1::SettleResponse::Success {
                payer: payment.from.to_string(),
                transaction: tx_hash.to_string(),
                network: payload.network.clone(),
                extensions: None,
            })
        })
    }

    fn supported(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<proto::SupportedResponse, FacilitatorError>> + Send + '_>>
    {
        Box::pin(async move {
            let chain_id = self.provider.chain_id();
            let kinds = {
                let mut kinds = Vec::with_capacity(1);
                let network = crate::networks::evm_network_registry().name_by_chain_id(&chain_id);
                if let Some(network) = network {
                    kinds.push(proto::SupportedPaymentKind {
                        x402_version: v1::V1.into(),
                        scheme: ExactScheme.to_string(),
                        network: network.to_string(),
                        extra: None,
                    });
                }
                kinds
            };
            let signers = {
                let mut signers = HashMap::with_capacity(1);
                signers.insert(
                    V1Eip155Exact.caip_family(),
                    self.provider.signer_addresses(),
                );
                signers
            };
            Ok(proto::SupportedResponse {
                kinds,
                extensions: Vec::new(),
                signers,
            })
        })
    }
}

/// Facilitator for V2 EIP-155 exact scheme payments.
///
/// V2 supports both EIP-3009 and Permit2 transfer methods. The transfer method
/// is determined by the [`ExactPayload`] variant in the payment payload.
pub struct V2Eip155ExactFacilitator<P> {
    provider: P,
}

impl<P> std::fmt::Debug for V2Eip155ExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2Eip155ExactFacilitator")
            .finish_non_exhaustive()
    }
}

impl<P> V2Eip155ExactFacilitator<P> {
    /// Creates a new V2 EIP-155 exact scheme facilitator with the given provider.
    pub const fn new(provider: P) -> Self {
        Self { provider }
    }
}

impl<P> Facilitator for V2Eip155ExactFacilitator<P>
where
    P: Eip155MetaTransactionProvider + ChainProvider + Send + Sync,
    P::Inner: Provider,
    Eip155ExactError: From<P::Error>,
{
    fn verify(
        &self,
        request: proto::VerifyRequest,
    ) -> Pin<Box<dyn Future<Output = Result<proto::VerifyResponse, FacilitatorError>> + Send + '_>>
    {
        Box::pin(async move {
            let request = types::v2::VerifyRequest::from_proto(request)?;
            let payload = &request.payment_payload;
            let requirements = &request.payment_requirements;
            match &payload.payload {
                ExactPayload::Eip3009(eip3009) => {
                    let (contract, payment, eip712_domain) = verify::assert_valid_v2_payment(
                        self.provider.inner(),
                        self.provider.chain(),
                        eip3009,
                        payload,
                        requirements,
                    )
                    .await?;
                    let payer =
                        verify_payment(self.provider.inner(), &contract, &payment, &eip712_domain)
                            .await?;
                    Ok(v2::VerifyResponse::valid(payer.to_string()))
                }
                ExactPayload::Permit2(permit2) => {
                    let (_erc20, payment, eip712_domain) = verify::assert_valid_v2_permit2_payment(
                        self.provider.inner(),
                        self.provider.chain(),
                        permit2,
                        payload,
                        requirements,
                    )
                    .await?;
                    let payer =
                        verify_permit2_payment(self.provider.inner(), &payment, &eip712_domain)
                            .await?;
                    Ok(v2::VerifyResponse::valid(payer.to_string()))
                }
            }
        })
    }

    fn settle(
        &self,
        request: proto::SettleRequest,
    ) -> Pin<Box<dyn Future<Output = Result<proto::SettleResponse, FacilitatorError>> + Send + '_>>
    {
        Box::pin(async move {
            let request = types::v2::SettleRequest::from_settle(request)?;
            let payload = &request.payment_payload;
            let requirements = &request.payment_requirements;
            match &payload.payload {
                ExactPayload::Eip3009(eip3009) => {
                    let (contract, payment, eip712_domain) = verify::assert_valid_v2_payment(
                        self.provider.inner(),
                        self.provider.chain(),
                        eip3009,
                        payload,
                        requirements,
                    )
                    .await?;
                    let tx_hash =
                        settle_payment(&self.provider, &contract, &payment, &eip712_domain).await?;

                    Ok(v2::SettleResponse::Success {
                        payer: payment.from.to_string(),
                        transaction: tx_hash.to_string(),
                        network: payload.accepted.network.to_string(),
                        extensions: None,
                    })
                }
                ExactPayload::Permit2(permit2) => {
                    let (_erc20, payment, _eip712_domain) =
                        verify::assert_valid_v2_permit2_payment(
                            self.provider.inner(),
                            self.provider.chain(),
                            permit2,
                            payload,
                            requirements,
                        )
                        .await?;
                    let tx_hash = settle_permit2_payment(&self.provider, &payment).await?;
                    Ok(v2::SettleResponse::Success {
                        payer: payment.from.to_string(),
                        transaction: tx_hash.to_string(),
                        network: payload.accepted.network.to_string(),
                        extensions: None,
                    })
                }
            }
        })
    }

    fn supported(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<proto::SupportedResponse, FacilitatorError>> + Send + '_>>
    {
        Box::pin(async move {
            let chain_id = self.provider.chain_id();
            let kinds = vec![proto::SupportedPaymentKind {
                x402_version: v2::V2.into(),
                scheme: ExactScheme.to_string(),
                network: chain_id.into(),
                extra: None,
            }];
            let signers = {
                let mut signers = HashMap::with_capacity(1);
                signers.insert(
                    V2Eip155Exact.caip_family(),
                    self.provider.signer_addresses(),
                );
                signers
            };
            Ok(proto::SupportedResponse {
                kinds,
                extensions: Vec::new(),
                signers,
            })
        })
    }
}
