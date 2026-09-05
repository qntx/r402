//! Facilitator-side verification and settlement for the EIP-155 upto scheme.
//!
//! Verify: `amount` = signed max. Settle: `amount` = actual ≤ signed max
//! (HTTP Sequential patches this from `UptoActualAmount`). Zero actual skips
//! the on-chain transaction.

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

mod contract;
mod settle;
mod verify;
mod verify_permit2;
mod verify_witness;

use std::collections::HashMap;
use std::future::Future;
use std::str::FromStr;

use alloy_primitives::{Address, U256};
use alloy_provider::Provider;
use compact_str::CompactString;
use r402_extensions::BuilderCodeFacilitatorExtension;
use r402_facilitator::{Duplicate, Facilitator, SettlementCache};
use r402_protocol::error::{FacilitatorError, VerificationError};
use r402_protocol::network::ChainProvider;
use r402_protocol::payment::{
    Extensions, SettleRequest, SettleResponse, SupportedPaymentKind, SupportedResponse, V2,
    VerifyRequest, VerifyResponse,
};
use r402_protocol::scheme::{SchemeId, UptoScheme};

use self::settle::{UptoSettleOutcome, settle_permit2_upto};
use self::verify::{
    PreparedUptoPermit2, assert_offchain_valid_settle, verify_permit2_upto_payment,
};
use crate::chain::Eip155MetaTransactionProvider;
use crate::eip2612::EIP2612_GAS_SPONSORING_KEY;
use crate::erc20_approval::ERC20_APPROVAL_GAS_SPONSORING_KEY;
use crate::error::Eip155ExactError;
use crate::upto::{Eip155Upto, payload};

/// Facilitator for the EIP-155 upto scheme (Permit2 only).
pub struct Eip155UptoFacilitator<P> {
    provider: P,
    clock_skew_tolerance: u64,
    settlement_cache: SettlementCache,
    builder_code: Option<BuilderCodeFacilitatorExtension>,
}

impl<P> Eip155UptoFacilitator<P> {
    /// Constructs a facilitator from a chain provider.
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
    pub const fn with_settlement_cache(provider: P, settlement_cache: SettlementCache) -> Self {
        Self {
            provider,
            clock_skew_tolerance: crate::EVM_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS,
            settlement_cache,
            builder_code: None,
        }
    }

    /// Appends an ERC-8021 Schema 2 suffix (`w`) on settlement transactions.
    #[must_use]
    pub fn with_builder_code(mut self, extension: BuilderCodeFacilitatorExtension) -> Self {
        self.builder_code = Some(extension);
        self
    }

    fn data_suffix(&self, extensions: &Extensions) -> Vec<u8> {
        self.builder_code
            .as_ref()
            .and_then(|ext| ext.build_data_suffix(extensions, 2))
            .unwrap_or_default()
    }

    /// Overrides the clock-skew tolerance (seconds).
    #[must_use]
    pub const fn with_clock_skew_tolerance(mut self, seconds: u64) -> Self {
        self.clock_skew_tolerance = seconds;
        self
    }

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

impl<P> Facilitator for Eip155UptoFacilitator<P>
where
    P: Eip155MetaTransactionProvider + ChainProvider + Send + Sync,
    P::Inner: Provider,
    Eip155ExactError: From<P::Error>,
{
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        let request = payload::v2::VerifyRequest::from_verify(request)?;
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
        Ok(VerifyResponse::valid(payer.to_string()))
    }

    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        let request = payload::v2::SettleRequest::from_settle(request)?;
        let signer_addresses = self.signer_addresses_typed();
        let nonce: U256 = request
            .payment_payload
            .payload
            .permit2_authorization
            .nonce
            .into();
        let cache_key = self.permit2_cache_key(nonce);
        if self.settlement_cache.reserve(cache_key.clone()) == Duplicate::Yes {
            return Err(VerificationError::DuplicateSettlement.into());
        }

        let outcome = self.broadcast_upto(&request, &signer_addresses).await;
        if should_release_cache(&outcome) {
            self.settlement_cache.release(&cache_key);
        }
        outcome
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
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
            SupportedPaymentKind::new(V2.into(), UptoScheme.to_string(), chain_id.to_string())
                .with_optional_extra(extra),
        ];
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::with_capacity(1);
        let _ = signers.insert(Eip155Upto.caip_family().into(), signer_strings);
        std::future::ready(Ok(SupportedResponse::new()
            .with_kinds(kinds)
            .with_signers(signers)
            .with_extensions(vec![
                EIP2612_GAS_SPONSORING_KEY.into(),
                ERC20_APPROVAL_GAS_SPONSORING_KEY.into(),
            ])))
    }
}

fn should_release_cache(outcome: &Result<SettleResponse, FacilitatorError>) -> bool {
    match outcome {
        Ok(SettleResponse::Success { .. }) => false,
        Ok(SettleResponse::Failure { transaction, .. }) => transaction.is_empty(),
        Ok(_) | Err(_) => true,
    }
}

impl<P> Eip155UptoFacilitator<P>
where
    P: Eip155MetaTransactionProvider + ChainProvider + Send + Sync,
    P::Inner: Provider,
    Eip155ExactError: From<P::Error>,
{
    async fn broadcast_upto(
        &self,
        request: &payload::v2::SettleRequest,
        signer_addresses: &[Address],
    ) -> Result<SettleResponse, FacilitatorError> {
        let actual_amount = assert_offchain_valid_settle(
            &request.payment_payload,
            &request.payment_requirements,
            signer_addresses,
            self.clock_skew_tolerance,
        )?;
        let prepared = PreparedUptoPermit2::try_new(
            *self.provider.chain(),
            &request.payment_payload.payload,
            &request.payment_payload.extensions,
        )?;
        let payer = prepared.from;
        let suffix = self.data_suffix(&request.payment_payload.extensions);
        let outcome =
            settle_permit2_upto(&self.provider, &prepared, actual_amount, &suffix).await?;
        let network = request.payment_payload.accepted.network.to_string();
        let transaction = match outcome {
            UptoSettleOutcome::ZeroSettle => CompactString::new(""),
            UptoSettleOutcome::OnChain(hash) => hash.to_string().into(),
        };
        Ok(SettleResponse::Success {
            payer: Some(payer.to_string().into()),
            transaction,
            network: network.into(),
            amount: Some(actual_amount.to_string().into()),
            extensions: Extensions::new(),
            extension_responses: Extensions::new(),
            extra: None,
        })
    }
}

impl<P> Eip155UptoFacilitator<P>
where
    P: ChainProvider,
{
    fn signer_addresses_typed(&self) -> Vec<Address> {
        self.provider
            .signer_addresses()
            .into_iter()
            .filter_map(|s| Address::from_str(&s).ok())
            .collect()
    }
}
