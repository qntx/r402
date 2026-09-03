//! Facilitator-side payment verification and settlement for the NEAR exact scheme.

mod settle;
mod settlement_cache;
mod verify;

use std::collections::HashMap;
use std::future::Future;

use compact_str::CompactString;
use r402_facilitator::Facilitator;
use r402_protocol::error::FacilitatorError;
use r402_protocol::network::ChainProvider;
use r402_protocol::payment::{
    SettleRequest, SettleResponse, SupportedPaymentKind, SupportedResponse, V2, VerifyRequest,
    VerifyResponse,
};
use r402_protocol::scheme::SchemeId;
pub use settle::settle_request;
pub use settlement_cache::{SettlementCache, settlement_cache_key};
pub use verify::{decode_signed_delegate, default_max_sponsored_gas, verify_request_json};

use crate::chain::NearChainProvider;
use crate::chain::rpc::{NearRpc, NearRpcError, NearSettlementOutcome};
use crate::exact::{ExactScheme, NearExact};

/// Operations the exact facilitator needs from a chain provider.
pub trait NearFacilitatorOps: NearRpc + ChainProvider + Clone + Send + Sync {
    /// Relayer account IDs this facilitator will sign for.
    fn relayer_ids(&self) -> Vec<String>;

    /// Maximum sponsored gas in gas units.
    fn max_sponsored_gas(&self) -> u64;

    /// Wrap a signed delegate in an outer relayer transaction and wait.
    fn submit_signed_delegate(
        &self,
        relayer_id: &str,
        signed_delegate_b64: &str,
    ) -> impl Future<Output = Result<NearSettlementOutcome, NearRpcError>> + Send;
}

impl NearFacilitatorOps for NearChainProvider {
    fn relayer_ids(&self) -> Vec<String> {
        Self::relayer_ids(self)
    }

    fn max_sponsored_gas(&self) -> u64 {
        Self::max_sponsored_gas(self)
    }

    fn submit_signed_delegate(
        &self,
        relayer_id: &str,
        signed_delegate_b64: &str,
    ) -> impl Future<Output = Result<NearSettlementOutcome, NearRpcError>> + Send {
        Self::submit_signed_delegate(self, relayer_id, signed_delegate_b64)
    }
}

/// Facilitator for NEAR exact scheme payments.
pub struct NearExactFacilitator<P> {
    provider: P,
    max_sponsored_gas: u64,
    settlement_cache: SettlementCache,
}

impl<P> NearExactFacilitator<P> {
    /// Constructs a facilitator from a chain provider.
    ///
    /// Copies [`NearFacilitatorOps::max_sponsored_gas`] and uses a default
    /// [`SettlementCache`].
    ///
    /// # Errors
    ///
    /// Currently infallible. [`Result`] so `try_new(provider)?` compiles.
    #[allow(
        clippy::unnecessary_wraps,
        reason = "Result so try_new(provider)? compiles"
    )]
    pub fn try_new(provider: P) -> Result<Self, FacilitatorError>
    where
        P: NearFacilitatorOps,
    {
        let gas = provider.max_sponsored_gas();
        Ok(Self::new(provider, gas))
    }

    /// Creates a facilitator with the default in-memory settlement cache.
    pub fn new(provider: P, max_sponsored_gas: u64) -> Self {
        Self::with_settlement_cache(provider, max_sponsored_gas, SettlementCache::new())
    }

    /// Creates a facilitator with a shared [`SettlementCache`].
    pub const fn with_settlement_cache(
        provider: P,
        max_sponsored_gas: u64,
        settlement_cache: SettlementCache,
    ) -> Self {
        Self {
            provider,
            max_sponsored_gas,
            settlement_cache,
        }
    }
}

impl<P> std::fmt::Debug for NearExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NearExactFacilitator")
            .field("max_sponsored_gas", &self.max_sponsored_gas)
            .finish_non_exhaustive()
    }
}

impl<P> Facilitator for NearExactFacilitator<P>
where
    P: NearFacilitatorOps + ChainProvider,
{
    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_near::exact::verify", skip_all)
    )]
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        let json = request.into_json();
        let expected_network = self.provider.chain_id().to_string();
        Ok(verify_request_json(
            &self.provider,
            &self.provider.relayer_ids(),
            self.max_sponsored_gas,
            &expected_network,
            &json,
        )
        .await)
    }

    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_near::exact::settle", skip_all)
    )]
    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        let json = request.into_json();
        let relayer_ids = self.provider.relayer_ids();
        let expected_network = self.provider.chain_id().to_string();
        let provider = self.provider.clone();
        Ok(settle_request(
            &self.provider,
            &relayer_ids,
            &self.settlement_cache,
            self.max_sponsored_gas,
            &expected_network,
            &json,
            move |relayer_id, signed_b64| async move {
                provider
                    .submit_signed_delegate(&relayer_id, &signed_b64)
                    .await
            },
        )
        .await)
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
        let chain_id = self.provider.chain_id();
        let kinds = vec![SupportedPaymentKind::new(
            V2.into(),
            ExactScheme.to_string(),
            chain_id.to_string(),
        )];
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::with_capacity(1);
        let _ = signers.insert(
            NearExact.caip_family().into(),
            self.provider
                .signer_addresses()
                .into_iter()
                .map(CompactString::from)
                .collect(),
        );
        std::future::ready(Ok(SupportedResponse::new()
            .with_kinds(kinds)
            .with_signers(signers)))
    }
}
