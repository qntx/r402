//! Facilitator-side payment verification and settlement for the Aptos exact scheme.

mod settle;
mod verify;

use std::collections::HashMap;
use std::future::Future;

use compact_str::CompactString;
use r402_facilitator::{Facilitator, SettlementCache};
use r402_protocol::error::FacilitatorError;
use r402_protocol::network::ChainProvider;
use r402_protocol::payment::{
    SettleRequest, SettleResponse, SupportedPaymentKind, SupportedResponse, V2, VerifyRequest,
    VerifyResponse,
};
use r402_protocol::scheme::SchemeId;
pub use settle::settle_request;
pub use verify::verify_request_json;

use crate::chain::{AptosChainProvider, AptosProviderError, AptosSimulationResult};
use crate::exact::{AptosExact, AptosExtra, ExactScheme};

/// Operations the exact facilitator needs from a chain provider.
pub trait AptosFacilitatorOps: ChainProvider + Send + Sync {
    /// Fee-payer addresses this facilitator will sign for (long form).
    fn fee_payer_addresses(&self) -> Vec<String>;

    /// Whether sponsorship is advertised.
    fn sponsor_transactions(&self) -> bool;

    /// Current FA balance of `owner` for `asset`.
    fn fungible_asset_balance(
        &self,
        owner: &str,
        asset: &str,
        network: &str,
    ) -> impl Future<Output = Result<u64, AptosProviderError>> + Send;

    /// Simulate the payment transaction.
    fn simulate_payment(
        &self,
        transaction_base64: &str,
        network: &str,
    ) -> impl Future<Output = Result<AptosSimulationResult, AptosProviderError>> + Send;

    /// Sign as fee payer when sponsored, submit, and wait.
    fn sign_and_submit(
        &self,
        transaction_base64: &str,
        fee_payer: Option<&str>,
        network: &str,
    ) -> impl Future<Output = Result<String, AptosProviderError>> + Send;
}

impl AptosFacilitatorOps for AptosChainProvider {
    fn fee_payer_addresses(&self) -> Vec<String> {
        Self::fee_payer_addresses(self)
    }

    fn sponsor_transactions(&self) -> bool {
        Self::sponsor_transactions(self)
    }

    fn fungible_asset_balance(
        &self,
        owner: &str,
        asset: &str,
        _network: &str,
    ) -> impl Future<Output = Result<u64, AptosProviderError>> + Send {
        Self::fungible_asset_balance(self, owner, asset)
    }

    fn simulate_payment(
        &self,
        transaction_base64: &str,
        _network: &str,
    ) -> impl Future<Output = Result<AptosSimulationResult, AptosProviderError>> + Send {
        Self::simulate_payment(self, transaction_base64)
    }

    fn sign_and_submit(
        &self,
        transaction_base64: &str,
        fee_payer: Option<&str>,
        _network: &str,
    ) -> impl Future<Output = Result<String, AptosProviderError>> + Send {
        Self::sign_and_submit(self, transaction_base64, fee_payer)
    }
}

/// Facilitator for Aptos exact scheme payments.
pub struct AptosExactFacilitator<P> {
    provider: P,
    sponsor_transactions: bool,
    settlement_cache: SettlementCache,
}

impl<P> AptosExactFacilitator<P> {
    /// Constructs a facilitator from a chain provider.
    ///
    /// Copies [`AptosFacilitatorOps::sponsor_transactions`] and uses a default
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
        P: AptosFacilitatorOps,
    {
        let sponsor = provider.sponsor_transactions();
        Ok(Self::new(provider, sponsor))
    }

    /// Creates a facilitator with the default in-memory settlement cache.
    pub fn new(provider: P, sponsor_transactions: bool) -> Self {
        Self::with_settlement_cache(provider, sponsor_transactions, SettlementCache::new())
    }

    /// Creates a facilitator with a shared [`SettlementCache`].
    pub const fn with_settlement_cache(
        provider: P,
        sponsor_transactions: bool,
        settlement_cache: SettlementCache,
    ) -> Self {
        Self {
            provider,
            sponsor_transactions,
            settlement_cache,
        }
    }
}

impl<P> std::fmt::Debug for AptosExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AptosExactFacilitator")
            .field("sponsor_transactions", &self.sponsor_transactions)
            .finish_non_exhaustive()
    }
}

impl<P> Facilitator for AptosExactFacilitator<P>
where
    P: AptosFacilitatorOps + ChainProvider,
{
    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_aptos::exact::verify", skip_all)
    )]
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        let json = request.into_json();
        Ok(verify_request_json(&self.provider, &json).await)
    }

    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_aptos::exact::settle", skip_all)
    )]
    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        let json = request.into_json();
        Ok(settle_request(&self.provider, &self.settlement_cache, &json).await)
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
        let chain_id = self.provider.chain_id();
        let extra = if self.sponsor_transactions {
            self.provider
                .fee_payer_addresses()
                .into_iter()
                .next()
                .and_then(|fee_payer| {
                    serde_json::to_value(AptosExtra {
                        fee_payer: fee_payer.parse().ok(),
                    })
                    .ok()
                })
        } else {
            None
        };
        let kinds = vec![
            SupportedPaymentKind::new(V2.into(), ExactScheme.to_string(), chain_id.to_string())
                .with_optional_extra(extra),
        ];
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::with_capacity(1);
        let _ = signers.insert(
            AptosExact.caip_family().into(),
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
