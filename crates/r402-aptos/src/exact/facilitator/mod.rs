//! Facilitator-side payment verification and settlement for the Aptos exact scheme.

mod settle;
mod verify;

use std::collections::HashMap;
use std::future::Future;

use compact_str::CompactString;
use r402_core::cache::SettlementCache;
use r402_core::chain::ChainProvider;
use r402_core::error::FacilitatorError;
use r402_core::facilitator::{DynFacilitator, Facilitator};
use r402_core::scheme::{SchemeBuilder, SchemeId};
use r402_core::wire;
use serde::Deserialize;
pub use settle::settle_request;
pub use verify::verify_request_json;

#[cfg(test)]
mod tests;

use crate::chain::{AptosChainProvider, AptosProviderError, AptosSimulationResult};
use crate::exact::{AptosExact, AptosExtra, ExactScheme};

/// Optional JSON config for [`AptosExact`] [`SchemeBuilder`].
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AptosExactFacilitatorConfig {
    /// Whether `/supported` advertises `extra.feePayer`. Defaults to `true`.
    #[serde(default = "default_sponsor")]
    pub sponsor_transactions: bool,
}

const fn default_sponsor() -> bool {
    true
}

impl Default for AptosExactFacilitatorConfig {
    fn default() -> Self {
        Self {
            sponsor_transactions: true,
        }
    }
}

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

impl SchemeBuilder<AptosChainProvider> for AptosExact {
    fn build(
        &self,
        provider: AptosChainProvider,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        let sponsor = match config {
            None => provider.sponsor_transactions(),
            Some(value) => {
                serde_json::from_value::<AptosExactFacilitatorConfig>(value)?.sponsor_transactions
            }
        };
        Ok(Box::new(AptosExactFacilitator::new(provider, sponsor)))
    }
}

impl SchemeBuilder<&AptosChainProvider> for AptosExact {
    fn build(
        &self,
        provider: &AptosChainProvider,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        SchemeBuilder::<AptosChainProvider>::build(self, provider.clone(), config)
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
    async fn verify(
        &self,
        request: wire::VerifyRequest,
    ) -> Result<wire::VerifyResponse, FacilitatorError> {
        let json = request.into_json();
        Ok(verify_request_json(&self.provider, &json).await)
    }

    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_aptos::exact::settle", skip_all)
    )]
    async fn settle(
        &self,
        request: wire::SettleRequest,
    ) -> Result<wire::SettleResponse, FacilitatorError> {
        let json = request.into_json();
        Ok(settle_request(&self.provider, &self.settlement_cache, &json).await)
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<wire::SupportedResponse, FacilitatorError>> + Send {
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
            wire::SupportedPaymentKind::new(
                wire::V2.into(),
                ExactScheme.to_string(),
                chain_id.to_string(),
            )
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
        std::future::ready(Ok(wire::SupportedResponse::new()
            .with_kinds(kinds)
            .with_signers(signers)))
    }
}
