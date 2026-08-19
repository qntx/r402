//! Facilitator-side verification and settlement for the EIP-155 upto scheme.
//!
//! See [module-level docs](super) for the protocol snapshot. The key logical
//! difference from the exact scheme is that settlement is phase-dependent on
//! `paymentRequirements.amount`:
//!
//! - verify: `amount` = signed max (equal to `authorization.permitted.amount`)
//! - settle: `amount` = actual charge (≤ signed max), may be `0`

mod contract;
mod settle;
mod verify;

use std::collections::HashMap;
use std::future::Future;
use std::str::FromStr;

use alloy_primitives::{Address, U256};
use alloy_provider::Provider;
use compact_str::CompactString;
use r402_core::cache::{Duplicate, SettlementCache};
use r402_core::chain::ChainProvider;
use r402_core::error::FacilitatorError;
use r402_core::error::VerificationError;
use r402_core::facilitator::{DynFacilitator, Facilitator};
use r402_core::scheme::{SchemeBuilder, SchemeId};
use r402_core::wire;

use self::settle::{UptoSettleOutcome, settle_permit2_upto};
use self::verify::{
    PreparedUptoPermit2, assert_offchain_valid_settle, verify_permit2_upto_payment,
};
use crate::chain::Eip155MetaTransactionProvider;
use crate::error::Eip155ExactError;
use crate::upto::{Eip155Upto, UptoScheme, types};

/// Facilitator for the EIP-155 upto scheme (Permit2 only).
///
/// Verify runs the off-chain invariants (amount, spender, recipient, time)
/// plus on-chain allowance / balance checks and a worst-case simulation of
/// `x402UptoPermit2Proxy.settle(...)` with the signed maximum amount.
///
/// Settle consumes the actual amount from
/// `paymentRequirements.amount` (the resource server's usage-dependent
/// charge), enforces `actual ≤ signed_max`, and takes the **zero-settle
/// shortcut** (no on-chain tx) when the charge is zero.
///
/// Maintains a [`SettlementCache`] keyed by `chain_id:permit2:nonce_hex`
/// so concurrent or replayed `/settle` calls within the on-chain replay
/// window short-circuit with [`VerificationError::DuplicateSettlement`]
/// before any RPC work happens.
pub struct Eip155UptoFacilitator<P> {
    provider: P,
    clock_skew_tolerance: u64,
    settlement_cache: SettlementCache,
}

impl<P> Eip155UptoFacilitator<P> {
    /// Creates a new upto facilitator with the spec-recommended defaults
    /// (6 s clock-skew, 2-minute settlement-cache TTL).
    pub fn new(provider: P) -> Self {
        Self::with_settlement_cache(provider, SettlementCache::new())
    }

    /// Creates a facilitator with a caller-supplied [`SettlementCache`].
    pub const fn with_settlement_cache(provider: P, settlement_cache: SettlementCache) -> Self {
        Self {
            provider,
            clock_skew_tolerance: crate::EVM_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS,
            settlement_cache,
        }
    }

    /// Overrides the clock-skew tolerance (seconds).
    #[must_use]
    pub const fn with_clock_skew_tolerance(mut self, seconds: u64) -> Self {
        self.clock_skew_tolerance = seconds;
        self
    }

    /// Builds a settlement-cache key for the upto Permit2 path.
    fn permit2_cache_key(&self, nonce: U256) -> String
    where
        P: ChainProvider,
    {
        format!("{}:upto:permit2:{nonce:#x}", self.provider.chain_id())
    }
}

impl<P> std::fmt::Debug for Eip155UptoFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Eip155UptoFacilitator")
            .finish_non_exhaustive()
    }
}

impl<P> SchemeBuilder<P> for Eip155Upto
where
    P: Eip155MetaTransactionProvider + ChainProvider + Send + Sync + 'static,
    Eip155ExactError: From<P::Error>,
{
    fn build(
        &self,
        provider: P,
        _config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(Eip155UptoFacilitator::new(provider)))
    }
}

impl<P> Facilitator for Eip155UptoFacilitator<P>
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
        let signer_addresses = self.signer_addresses_typed();
        let payer = verify_permit2_upto_payment(
            self.provider.inner(),
            *self.provider.chain(),
            &request.payment_payload,
            &request.payment_requirements,
            &signer_addresses,
            self.clock_skew_tolerance,
        )
        .await?;
        Ok(wire::VerifyResponse::valid(payer.to_string()))
    }

    async fn settle(
        &self,
        request: wire::SettleRequest,
    ) -> Result<wire::SettleResponse, FacilitatorError> {
        let request = types::v2::SettleRequest::from_settle(request)?;
        let signer_addresses = self.signer_addresses_typed();
        // F-073: deduplicate before any RPC work.
        let nonce: U256 = request
            .payment_payload
            .payload
            .permit2_authorization
            .nonce
            .into();
        let cache_key = self.permit2_cache_key(nonce);
        if self.settlement_cache.reserve(cache_key) == Duplicate::Yes {
            return Err(VerificationError::DuplicateSettlement.into());
        }

        let actual_amount = assert_offchain_valid_settle(
            &request.payment_payload,
            &request.payment_requirements,
            &signer_addresses,
            self.clock_skew_tolerance,
        )?;

        let prepared = PreparedUptoPermit2::try_new(
            *self.provider.chain(),
            &request.payment_payload.payload,
            &request.payment_payload.extensions,
        )?;
        let payer = prepared.from;
        let outcome = settle_permit2_upto(&self.provider, &prepared, actual_amount).await?;

        let network = request.payment_payload.accepted.network.to_string();
        let transaction = match outcome {
            UptoSettleOutcome::ZeroSettle => CompactString::new(""),
            UptoSettleOutcome::OnChain(hash) => hash.to_string().into(),
        };

        Ok(wire::SettleResponse::Success {
            payer: payer.to_string().into(),
            transaction,
            network: network.into(),
            amount: Some(actual_amount.to_string().into()),
            extensions: wire::Extensions::new(),
        })
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<wire::SupportedResponse, FacilitatorError>> + Send {
        let chain_id = self.provider.chain_id();
        let signer_strings: Vec<CompactString> = self
            .provider
            .signer_addresses()
            .into_iter()
            .map(CompactString::from)
            .collect();
        let extra = signer_strings.first().map(|addr| {
            serde_json::json!({
                "assetTransferMethod": "permit2",
                "facilitatorAddress": addr,
            })
        });
        let kinds = vec![
            wire::SupportedPaymentKind::new(
                wire::V2.into(),
                UptoScheme.to_string(),
                chain_id.to_string(),
            )
            .with_optional_extra(extra),
        ];
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::with_capacity(1);
        let _ = signers.insert(Eip155Upto.caip_family().into(), signer_strings);
        std::future::ready(Ok(wire::SupportedResponse::new()
            .with_kinds(kinds)
            .with_signers(signers)))
    }
}

impl<P> Eip155UptoFacilitator<P>
where
    P: ChainProvider,
{
    /// Returns the facilitator's signer addresses parsed as [`Address`].
    ///
    /// Strings emitted by [`ChainProvider::signer_addresses`] that fail to
    /// parse are silently skipped, mirroring the wire-level
    /// `VecSkipError` policy used elsewhere.
    fn signer_addresses_typed(&self) -> Vec<Address> {
        self.provider
            .signer_addresses()
            .into_iter()
            .filter_map(|s| Address::from_str(&s).ok())
            .collect()
    }
}

/// Returns [`VerificationError::FacilitatorMismatch`] when `witness.facilitator`
/// is not in the facilitator's signer set.
pub(super) fn assert_facilitator_authorized(
    witness_facilitator: Address,
    signer_addresses: &[Address],
) -> Result<(), VerificationError> {
    if signer_addresses.contains(&witness_facilitator) {
        Ok(())
    } else {
        Err(VerificationError::UptoFacilitatorMismatch {
            witness: witness_facilitator.to_checksum(None),
        })
    }
}
