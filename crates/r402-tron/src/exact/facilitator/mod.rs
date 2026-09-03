//! Facilitator-side payment verification and settlement for the Tron exact scheme.
//!
//! Supports EIP-3009 (`transferWithAuthorization`) and SUN.io Permit2 via
//! `x402ExactPermit2Proxy`, routing on [`ExactPayload`].
//!
//! - EOA-only signature verification (Tron has no contract wallets)
//! - Balance and amount validation via [`crate::chain::TronGridClient`]
//! - TIP-712 domain construction
//! - On-chain settlement with transaction confirmation polling

use std::collections::HashMap;
use std::future::Future;

use alloy_primitives::{Address as EvmAddress, B256, Bytes, U256};
use r402_facilitator::{Duplicate, Facilitator, SettlementCache};
use r402_protocol::error::{FacilitatorError, VerificationError};
use r402_protocol::network::ChainProvider;
use r402_protocol::payment as wire;
use r402_protocol::payment::UnixTimestamp;
use r402_protocol::scheme::{ExactScheme, SchemeId};

use crate::chain::TronChainProvider;
use crate::exact::{ExactPayload, TronExact, TronExactError, payload};

mod settle;
mod signature;
mod verify;

use settle::{settle_payment, settle_permit2_payment};
pub use signature::TronSignatureError;
use verify::{verify_payment, verify_permit2_payment};

/// A fully specified EIP-3009 authorization payload for Tron settlement.
#[derive(Debug)]
pub struct Eip3009Payment {
    /// Authorized sender (`from`) — EOA.
    pub from: EvmAddress,
    /// Authorized recipient (`to`).
    pub to: EvmAddress,
    /// Transfer amount (token units).
    pub value: U256,
    /// Not valid before this timestamp (inclusive).
    pub valid_after: UnixTimestamp,
    /// Not valid at/after this timestamp (exclusive).
    pub valid_before: UnixTimestamp,
    /// Unique 32-byte nonce (prevents replay).
    pub nonce: B256,
    /// Raw signature bytes (EOA secp256k1, 65 bytes).
    pub signature: Bytes,
}

/// A fully specified Permit2 authorization payload for Tron settlement.
#[derive(Debug)]
pub struct Permit2Payment {
    /// Signer / owner address.
    pub from: EvmAddress,
    /// Destination address for funds.
    pub to: EvmAddress,
    /// Token contract address.
    pub token: EvmAddress,
    /// Permitted amount (token units).
    pub amount: U256,
    /// Must be the `x402ExactPermit2Proxy` address.
    pub spender: EvmAddress,
    /// Unique nonce (uint256).
    pub nonce: U256,
    /// Signature expires after this unix timestamp.
    pub deadline: U256,
    /// Payment invalid before this timestamp.
    pub valid_after: U256,
    /// EIP-712 signature bytes (EOA, 65 bytes).
    pub signature: Bytes,
}

/// Facilitator for Tron exact scheme payments.
///
/// Supports both EIP-3009 and Permit2 transfer methods. The transfer method
/// is determined by the [`ExactPayload`] variant in the payment payload.
///
/// Maintains a [`SettlementCache`] keyed by `chain_id:nonce_hex` so that
/// concurrent or replayed `/settle` calls return
/// [`VerificationError::DuplicateSettlement`] before the transaction is
/// broadcast a second time.
pub struct TronExactFacilitator {
    provider: TronChainProvider,
    clock_skew_tolerance: u64,
    settlement_cache: SettlementCache,
}

impl TronExactFacilitator {
    /// Constructs a facilitator from a chain provider.
    ///
    /// Uses [`crate::TRON_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS`] (6 s) for
    /// time-window validation and a default [`SettlementCache`] with the
    /// spec-recommended 2-minute TTL.
    ///
    /// # Errors
    ///
    /// Currently infallible. [`Result`] so `try_new(provider)?` compiles.
    #[allow(
        clippy::unnecessary_wraps,
        reason = "Result so try_new(provider)? compiles"
    )]
    pub fn try_new(provider: TronChainProvider) -> Result<Self, FacilitatorError> {
        Ok(Self::with_settlement_cache(
            provider,
            SettlementCache::new(),
        ))
    }

    /// Creates a facilitator with a caller-supplied [`SettlementCache`].
    #[must_use]
    pub const fn with_settlement_cache(
        provider: TronChainProvider,
        settlement_cache: SettlementCache,
    ) -> Self {
        Self {
            provider,
            clock_skew_tolerance: crate::TRON_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS,
            settlement_cache,
        }
    }

    /// Sets a custom clock-skew tolerance (in seconds) for time-window checks.
    #[must_use]
    pub const fn with_clock_skew_tolerance(mut self, seconds: u64) -> Self {
        self.clock_skew_tolerance = seconds;
        self
    }

    /// Builds a stable settlement-cache key for the EIP-3009 path.
    fn eip3009_cache_key(&self, nonce: B256) -> String {
        format!("{}:{nonce}", self.provider.chain_id())
    }

    /// Builds a stable settlement-cache key for the Permit2 path.
    fn permit2_cache_key(&self, nonce: &U256) -> String {
        format!("{}:permit2:{nonce:#x}", self.provider.chain_id())
    }
}

impl std::fmt::Debug for TronExactFacilitator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TronExactFacilitator")
            .finish_non_exhaustive()
    }
}

impl From<TronExactError> for FacilitatorError {
    fn from(e: TronExactError) -> Self {
        match e {
            TronExactError::SignatureRecovery(_)
            | TronExactError::SignerMismatch
            | TronExactError::InsufficientBalance
            | TronExactError::NotYetValid
            | TronExactError::Expired
            | TronExactError::ValueMismatch
            | TronExactError::RecipientMismatch
            | TronExactError::AssetMismatch
            | TronExactError::InvalidPermit2Spender(_)
            | TronExactError::NonceAlreadyUsed
            | TronExactError::ChainMismatch
            | TronExactError::MissingTip712Domain
            | TronExactError::UnsupportedTransferMethod => {
                Self::Verification(VerificationError::from(e))
            }
            TronExactError::TronGrid(_)
            | TronExactError::TransactionFailed(_)
            | TronExactError::ConfirmationTimeout => Self::Onchain(e.to_string()),
        }
    }
}

impl Facilitator for TronExactFacilitator {
    async fn verify(
        &self,
        request: wire::VerifyRequest,
    ) -> Result<wire::VerifyResponse, FacilitatorError> {
        let request = payload::v2::VerifyRequest::from_verify(request)?;
        let payload = &request.payment_payload;
        let requirements = &request.payment_requirements;
        let chain = self.provider.chain_reference();
        match &payload.payload {
            ExactPayload::Eip3009(eip3009) => {
                let (payment, eip712_domain) = verify::assert_valid_payment(
                    &self.provider,
                    &chain,
                    eip3009,
                    payload,
                    requirements,
                    self.clock_skew_tolerance,
                )
                .await?;
                let payer = verify_payment(&self.provider, &payment, &eip712_domain).await?;
                Ok(wire::VerifyResponse::valid(
                    crate::chain::Address::from_evm(payer).to_string(),
                ))
            }
            ExactPayload::Permit2(permit2) => {
                let (payment, eip712_domain) = verify::assert_valid_permit2_payment(
                    &self.provider,
                    &chain,
                    permit2,
                    payload,
                    requirements,
                    self.clock_skew_tolerance,
                )
                .await?;
                let payer =
                    verify_permit2_payment(&self.provider, &chain, &payment, &eip712_domain)
                        .await?;
                Ok(wire::VerifyResponse::valid(
                    crate::chain::Address::from_evm(payer).to_string(),
                ))
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
        let chain = self.provider.chain_reference();

        let cache_key = match &payload.payload {
            ExactPayload::Eip3009(eip3009) => self.eip3009_cache_key(eip3009.authorization.nonce),
            ExactPayload::Permit2(permit2) => {
                self.permit2_cache_key(&permit2.permit2_authorization.nonce.into())
            }
        };
        if self.settlement_cache.reserve(cache_key) == Duplicate::Yes {
            return Err(VerificationError::DuplicateSettlement.into());
        }

        match &payload.payload {
            ExactPayload::Eip3009(eip3009) => {
                let (payment, eip712_domain) = verify::assert_valid_payment(
                    &self.provider,
                    &chain,
                    eip3009,
                    payload,
                    requirements,
                    self.clock_skew_tolerance,
                )
                .await?;
                let tx_hash = settle_payment(&self.provider, &payment, &eip712_domain).await?;
                Ok(wire::SettleResponse::Success {
                    payer: Some(
                        crate::chain::Address::from_evm(payment.from)
                            .to_string()
                            .into(),
                    ),
                    transaction: tx_hash.into(),
                    network: payload.accepted.network.to_string().into(),
                    amount: Some(requirements.amount.0.to_string().into()),
                    extensions: wire::Extensions::new(),
                    extension_responses: wire::Extensions::new(),
                    extra: None,
                })
            }
            ExactPayload::Permit2(permit2) => {
                let (payment, _eip712_domain) = verify::assert_valid_permit2_payment(
                    &self.provider,
                    &chain,
                    permit2,
                    payload,
                    requirements,
                    self.clock_skew_tolerance,
                )
                .await?;
                let tx_hash = settle_permit2_payment(&self.provider, &chain, &payment).await?;
                Ok(wire::SettleResponse::Success {
                    payer: Some(
                        crate::chain::Address::from_evm(payment.from)
                            .to_string()
                            .into(),
                    ),
                    transaction: tx_hash.into(),
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
            TronExact.caip_family().into(),
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
