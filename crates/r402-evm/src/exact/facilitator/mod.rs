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
use std::future::Future;

use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_provider::Provider;
use r402_facilitator::{Duplicate, Facilitator, SettlementCache};
use r402_protocol::error::{FacilitatorError, VerificationError};
use r402_protocol::network::ChainProvider;
use r402_protocol::payment as wire;
use r402_protocol::payment::UnixTimestamp;
use r402_protocol::scheme::{ExactScheme, SchemeId};

use crate::chain::Eip155MetaTransactionProvider;
use crate::exact::{Eip155Exact, ExactPayload, payload};

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
#[allow(
    unused_macros,
    reason = "expanded only when traced! keeps the span (telemetry)"
)]
macro_rules! transfer_span {
    ($name:expr, $call:expr $(, $key:ident = $val:expr)*) => {{
        #[cfg(feature = "telemetry")]
        {
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
        }
        #[cfg(not(feature = "telemetry"))]
        {
            let _ = &$call;
            ()
        }
    }};
}

pub(crate) mod contract;
mod settle;
pub(crate) mod signature;
mod verify;

use settle::{settle_payment, settle_permit2_payment};
use verify::{verify_payment, verify_permit2_payment};

use crate::error::Eip155ExactError;

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

/// Facilitator for EIP-155 exact scheme payments.
///
/// Supports both EIP-3009 and Permit2 transfer methods. The transfer method
/// is determined by the [`ExactPayload`] variant in the payment payload.
///
/// Maintains a [`SettlementCache`] keyed by `chain_id:nonce_hex` so that
/// concurrent or replayed `/settle` calls within the on-chain replay
/// window return [`VerificationError::DuplicateSettlement`] before the
/// transaction is broadcast a second time. EIP-3009 and Permit2 nonces
/// are already unique per buyer + token + chain, so this cache mirrors
/// the underlying chain's protection but reacts faster than RPC
/// confirmation.
pub struct Eip155ExactFacilitator<P> {
    provider: P,
    /// Grace period (in seconds) applied to time-window checks to tolerate
    /// clock drift between the facilitator and the blockchain network.
    clock_skew_tolerance: u64,
    /// Settle-time deduplication cache (see struct-level docs).
    settlement_cache: SettlementCache,
}

impl<P> Eip155ExactFacilitator<P> {
    /// Constructs a facilitator from a chain provider.
    ///
    /// Uses [`crate::EVM_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS`] (6 s, one EVM
    /// block, aligned with Go's `Permit2DeadlineBuffer`) for time-window
    /// validation, and a default [`SettlementCache`] with the spec-recommended
    /// 2-minute TTL.
    ///
    /// # Errors
    ///
    /// Currently infallible. [`Result`] so `try_new(provider)?` compiles.
    #[allow(
        clippy::unnecessary_wraps,
        reason = "Result so try_new(provider)? compiles"
    )]
    pub fn try_new(provider: P) -> Result<Self, FacilitatorError> {
        Ok(Self::with_settlement_cache(
            provider,
            SettlementCache::new(),
        ))
    }

    /// Creates a facilitator with a caller-supplied [`SettlementCache`].
    ///
    /// Use this when you want to share a cache across worker tasks (e.g.
    /// behind a load-balancer that pins by buyer address) or to plug in a
    /// custom backend (Redis, etc.) by wrapping its primitives in the same
    /// shape.
    pub const fn with_settlement_cache(provider: P, settlement_cache: SettlementCache) -> Self {
        Self {
            provider,
            clock_skew_tolerance: crate::EVM_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS,
            settlement_cache,
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

    /// Builds a stable settlement-cache key for the EIP-3009 path.
    ///
    /// `B256` formats as `0x`-prefixed lowercase hex, matching the
    /// `eip155:<chain>:<nonce>` convention used by the Go SDK.
    fn eip3009_cache_key(&self, nonce: B256) -> String
    where
        P: ChainProvider,
    {
        format!("{}:{nonce}", self.provider.chain_id())
    }

    /// Builds a stable settlement-cache key for the Permit2 path.
    ///
    /// Permit2 nonces are `uint256`; the `:permit2:` infix discriminates
    /// from EIP-3009 32-byte nonces so the two namespaces never collide
    /// on a value that happens to be representable in both.
    fn permit2_cache_key(&self, nonce: U256) -> String
    where
        P: ChainProvider,
    {
        format!("{}:permit2:{nonce:#x}", self.provider.chain_id())
    }
}

impl<P> std::fmt::Debug for Eip155ExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Eip155ExactFacilitator")
            .finish_non_exhaustive()
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
        let request = payload::v2::VerifyRequest::from_verify(request)?;
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
        let request = payload::v2::SettleRequest::from_settle(request)?;
        let payload = &request.payment_payload;
        let requirements = &request.payment_requirements;
        // F-073: deduplicate concurrent / replayed settle attempts before
        // any verification or RPC work. EIP-3009 nonces and Permit2 nonces
        // are already unique per buyer + token + chain on-chain, but
        // checking the cache up-front protects against double broadcast
        // during the confirmation window where the chain has not yet
        // observed the first transaction.
        let cache_key = match &payload.payload {
            ExactPayload::Eip3009(eip3009) => self.eip3009_cache_key(eip3009.authorization.nonce),
            ExactPayload::Permit2(permit2) => {
                self.permit2_cache_key(permit2.permit2_authorization.nonce.into())
            }
        };
        if self.settlement_cache.reserve(cache_key) == Duplicate::Yes {
            return Err(VerificationError::DuplicateSettlement.into());
        }
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
                    extension_responses: wire::Extensions::new(),
                    extra: None,
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
                    extension_responses: wire::Extensions::new(),
                    extra: None,
                })
            }
        }
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<wire::SupportedResponse, FacilitatorError>> + Send {
        use compact_str::CompactString;

        let chain_id = self.provider.chain_id();
        let kinds = vec![wire::SupportedPaymentKind::new(
            wire::V2.into(),
            ExactScheme.to_string(),
            chain_id.to_string(),
        )];
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::with_capacity(1);
        let _ = signers.insert(
            Eip155Exact.caip_family().into(),
            self.provider
                .signer_addresses()
                .into_iter()
                .map(CompactString::from)
                .collect(),
        );
        std::future::ready(Ok(wire::SupportedResponse::new()
            .with_kinds(kinds)
            .with_signers(signers)))
    }
}
