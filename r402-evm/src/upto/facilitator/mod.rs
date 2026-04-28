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
use std::str::FromStr;

use alloy_primitives::Address;
use alloy_provider::Provider;
use compact_str::CompactString;
use r402_core::chain::ChainProvider;
use r402_core::error::VerificationError;
use r402_core::facilitator::{DynFacilitator, Facilitator, FacilitatorError};
use r402_core::scheme::{SchemeBuilder, SchemeId};
use r402_core::wire;

use self::settle::{UptoSettleOutcome, settle_permit2_upto};
use self::verify::{
    PreparedUptoPermit2, assert_offchain_valid_settle, verify_permit2_upto_payment,
};
use crate::chain::Eip155MetaTransactionProvider;
use crate::exact::facilitator::Eip155ExactError;
use crate::upto::{Eip155Upto, UptoScheme, types};

/// Default clock skew tolerance (seconds) for time-window checks.
///
/// Aliases [`crate::EVM_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS`] (6 s) so the
/// upto and exact facilitators share a single source of truth aligned with
/// Go's `Permit2DeadlineBuffer`.
const DEFAULT_CLOCK_SKEW_TOLERANCE: u64 = crate::EVM_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS;

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
pub struct Eip155UptoFacilitator<P> {
    provider: P,
    clock_skew_tolerance: u64,
}

impl<P> Eip155UptoFacilitator<P> {
    /// Creates a new upto facilitator with default clock-skew tolerance (30 s).
    pub const fn new(provider: P) -> Self {
        Self {
            provider,
            clock_skew_tolerance: DEFAULT_CLOCK_SKEW_TOLERANCE,
        }
    }

    /// Overrides the clock-skew tolerance (seconds).
    #[must_use]
    pub const fn with_clock_skew_tolerance(mut self, seconds: u64) -> Self {
        self.clock_skew_tolerance = seconds;
        self
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

    async fn supported(&self) -> Result<wire::SupportedResponse, FacilitatorError> {
        let chain_id = self.provider.chain_id();
        let signer_strings: Vec<CompactString> = self
            .provider
            .signer_addresses()
            .into_iter()
            .map(CompactString::from)
            .collect();
        // Spec §2: publish the first signer as `extra.facilitatorAddress`
        // so the resource-server price tag and client signing path can pick
        // it up via `enrich(supported)`. Mirrors the official TypeScript /
        // Go behaviour (random pick is not required for correctness; first
        // is deterministic and avoids surprising the buyer between calls).
        let extra = signer_strings.first().map(|addr| {
            serde_json::json!({
                "assetTransferMethod": "permit2",
                "facilitatorAddress": addr,
            })
        });
        let kinds = vec![wire::SupportedPaymentKind {
            x402_version: wire::V2.into(),
            scheme: UptoScheme.to_string().into(),
            network: chain_id.to_string().into(),
            extra,
        }];
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::with_capacity(1);
        let _ = signers.insert(Eip155Upto.caip_family().into(), signer_strings);
        Ok(wire::SupportedResponse {
            kinds,
            extensions: Vec::new(),
            signers,
        })
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
