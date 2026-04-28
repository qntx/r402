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

use std::collections::HashMap;

use alloy_primitives::{Address, B256, Bytes, U256, address};
use alloy_provider::Provider;
use r402_core::chain::ChainProvider;
use r402_core::facilitator::{DynFacilitator, Facilitator, FacilitatorError};
use r402_core::scheme::{SchemeBuilder, SchemeId};
use r402_core::wire;
use r402_core::wire::UnixTimestamp;

use crate::chain::Eip155MetaTransactionProvider;
use crate::exact::types;
use crate::exact::{Eip155Exact, ExactPayload, ExactScheme};

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

/// Creates a tracing span for a `transferWithAuthorization` call.
///
/// All EIP-3009 call sites share the same set of span fields; this macro
/// avoids repeating the field list at every call site.
macro_rules! transfer_span {
    ($name:expr, $call:expr $(, $key:ident = $val:expr)*) => {
        tracing::info_span!($name,
            from = %$call.from,
            to = %$call.to,
            value = %$call.value,
            valid_after = %$call.valid_after,
            valid_before = %$call.valid_before,
            nonce = %$call.nonce,
            signature = %$call.signature,
            token_contract = %$call.contract_address,
            $($key = $val,)*
            otel.kind = "client",
        )
    };
}

pub(crate) mod contract;
mod error;
mod settle;
pub(crate) mod signature;
mod verify;

pub use error::Eip155ExactError;
use settle::{settle_payment, settle_permit2_payment};
pub use signature::StructuredSignatureFormatError;
pub(crate) use verify::assert_time;
use verify::{verify_payment, verify_permit2_payment};

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
    /// EIP-712 signature bytes.
    pub signature: Bytes,
}

/// Default clock skew tolerance in seconds for time validation.
///
/// Applied as a grace buffer when checking `validBefore` / `deadline` expiration
/// and `validAfter` early-arrival, to account for clock drift between the
/// facilitator host and the blockchain network.
///
/// Re-exported from the crate root for shared use with the upto facilitator;
/// kept as a `const` alias here for backwards compatibility with downstream
/// code that imports the module-private constant.
const DEFAULT_CLOCK_SKEW_TOLERANCE: u64 = crate::EVM_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS;

/// Facilitator for EIP-155 exact scheme payments.
///
/// Supports both EIP-3009 and Permit2 transfer methods. The transfer method
/// is determined by the [`ExactPayload`] variant in the payment payload.
pub struct Eip155ExactFacilitator<P> {
    provider: P,
    /// Grace period (in seconds) applied to time-window checks to tolerate
    /// clock drift between the facilitator and the blockchain network.
    clock_skew_tolerance: u64,
}

impl<P> Eip155ExactFacilitator<P> {
    /// Creates a new EIP-155 exact scheme facilitator with the given provider.
    ///
    /// Uses [`crate::EVM_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS`] (6 s, one EVM
    /// block, aligned with Go's `Permit2DeadlineBuffer`) for time-window
    /// validation.
    pub const fn new(provider: P) -> Self {
        Self {
            provider,
            clock_skew_tolerance: DEFAULT_CLOCK_SKEW_TOLERANCE,
        }
    }

    /// Sets a custom clock-skew tolerance (in seconds) for time-window checks.
    ///
    /// A larger value is more lenient toward clock drift between the facilitator
    /// and the chain; a value of `0` enforces exact-time boundaries.
    #[must_use]
    pub const fn with_clock_skew_tolerance(mut self, seconds: u64) -> Self {
        self.clock_skew_tolerance = seconds;
        self
    }
}

impl<P> std::fmt::Debug for Eip155ExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Eip155ExactFacilitator")
            .finish_non_exhaustive()
    }
}

impl<P> SchemeBuilder<P> for Eip155Exact
where
    P: Eip155MetaTransactionProvider + ChainProvider + Send + Sync + 'static,
    Eip155ExactError: From<P::Error>,
{
    fn build(
        &self,
        provider: P,
        _config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(Eip155ExactFacilitator::new(provider)))
    }
}

impl<P> Facilitator for Eip155ExactFacilitator<P>
where
    P: Eip155MetaTransactionProvider + ChainProvider + Send + Sync,
    P::Inner: Provider,
    Eip155ExactError: From<P::Error>,
{
    async fn verify(
        &self,
        request: wire::VerifyRequest,
    ) -> Result<wire::VerifyResponse, FacilitatorError> {
        let request = types::v2::VerifyRequest::from_verify(request)?;
        let payload = &request.payment_payload;
        let requirements = &request.payment_requirements;
        match &payload.payload {
            ExactPayload::Eip3009(eip3009) => {
                let (contract, payment, eip712_domain) = verify::assert_valid_payment(
                    self.provider.inner(),
                    self.provider.chain(),
                    eip3009,
                    payload,
                    requirements,
                    self.clock_skew_tolerance,
                )
                .await?;
                let payer =
                    verify_payment(self.provider.inner(), &contract, &payment, &eip712_domain)
                        .await?;
                Ok(wire::VerifyResponse::valid(payer.to_string()))
            }
            ExactPayload::Permit2(permit2) => {
                let (_erc20, payment, eip712_domain) = verify::assert_valid_permit2_payment(
                    self.provider.inner(),
                    self.provider.chain(),
                    permit2,
                    payload,
                    requirements,
                    self.clock_skew_tolerance,
                )
                .await?;
                let payer =
                    verify_permit2_payment(self.provider.inner(), &payment, &eip712_domain).await?;
                Ok(wire::VerifyResponse::valid(payer.to_string()))
            }
        }
    }

    async fn settle(
        &self,
        request: wire::SettleRequest,
    ) -> Result<wire::SettleResponse, FacilitatorError> {
        let request = types::v2::SettleRequest::from_settle(request)?;
        let payload = &request.payment_payload;
        let requirements = &request.payment_requirements;
        match &payload.payload {
            ExactPayload::Eip3009(eip3009) => {
                let (contract, payment, eip712_domain) = verify::assert_valid_payment(
                    self.provider.inner(),
                    self.provider.chain(),
                    eip3009,
                    payload,
                    requirements,
                    self.clock_skew_tolerance,
                )
                .await?;
                let tx_hash =
                    settle_payment(&self.provider, &contract, &payment, &eip712_domain).await?;
                Ok(wire::SettleResponse::Success {
                    payer: payment.from.to_string().into(),
                    transaction: tx_hash.to_string().into(),
                    network: payload.accepted.network.to_string().into(),
                    amount: Some(requirements.amount.0.to_string().into()),
                    extensions: wire::Extensions::new(),
                })
            }
            ExactPayload::Permit2(permit2) => {
                let (_erc20, payment, _eip712_domain) = verify::assert_valid_permit2_payment(
                    self.provider.inner(),
                    self.provider.chain(),
                    permit2,
                    payload,
                    requirements,
                    self.clock_skew_tolerance,
                )
                .await?;
                let tx_hash = settle_permit2_payment(&self.provider, &payment).await?;
                Ok(wire::SettleResponse::Success {
                    payer: payment.from.to_string().into(),
                    transaction: tx_hash.to_string().into(),
                    network: payload.accepted.network.to_string().into(),
                    amount: Some(requirements.amount.0.to_string().into()),
                    extensions: wire::Extensions::new(),
                })
            }
        }
    }

    async fn supported(&self) -> Result<wire::SupportedResponse, FacilitatorError> {
        use compact_str::CompactString;

        let chain_id = self.provider.chain_id();
        let kinds = vec![wire::SupportedPaymentKind {
            x402_version: wire::V2.into(),
            scheme: ExactScheme.to_string().into(),
            network: chain_id.to_string().into(),
            extra: None,
        }];
        let signers = {
            let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::with_capacity(1);
            let _ = signers.insert(
                Eip155Exact.caip_family().into(),
                self.provider
                    .signer_addresses()
                    .into_iter()
                    .map(CompactString::from)
                    .collect(),
            );
            signers
        };
        Ok(wire::SupportedResponse {
            kinds,
            extensions: Vec::new(),
            signers,
        })
    }
}
