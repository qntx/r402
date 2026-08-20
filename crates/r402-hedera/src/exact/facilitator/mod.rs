//! Facilitator-side payment verification and settlement for the Hedera exact scheme.

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

use crate::chain::rpc::{HederaAccountResolution, HederaPreflightResult, HederaSignatureResult};
use crate::chain::{HederaChainProvider, HederaProviderError};
use crate::exact::{ExactScheme, HederaExact, HederaExtra};

/// Alias handling for `payTo` (facilitator config, not wire extra).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AliasPolicy {
    /// Reject aliases and unresolved destinations (default).
    #[default]
    Reject,
    /// Allow transfers that would create an account from an alias.
    Allow,
}

/// Optional JSON config for [`HederaExact`] [`SchemeBuilder`].
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HederaExactFacilitatorConfig {
    /// Alias policy. Defaults to [`AliasPolicy::Reject`].
    #[serde(default)]
    pub alias_policy: AliasPolicy,
}

/// Operations the exact facilitator needs from a chain provider.
pub trait HederaFacilitatorOps: ChainProvider + Send + Sync {
    /// Fee-payer account ids this facilitator will sign for.
    fn fee_payer_ids(&self) -> Vec<String>;

    /// Verify that `payer` signed the frozen transaction body.
    fn verify_payer_signature(
        &self,
        payer: &str,
        transaction_base64: &str,
        network: &str,
    ) -> impl Future<Output = Result<HederaSignatureResult, HederaProviderError>> + Send;

    /// Balance and token-association preflight.
    fn preflight_transfer(
        &self,
        payer: &str,
        pay_to: &str,
        asset: &str,
        amount: &str,
        network: &str,
    ) -> impl Future<Output = Result<HederaPreflightResult, HederaProviderError>> + Send;

    /// Resolve `payTo` for alias policy.
    fn resolve_account(
        &self,
        account_id_or_alias: &str,
        network: &str,
    ) -> impl Future<Output = Result<HederaAccountResolution, HederaProviderError>> + Send;

    /// Sign as fee payer, execute, and wait for a `SUCCESS` receipt.
    fn sign_and_submit(
        &self,
        transaction_base64: &str,
        fee_payer: &str,
        network: &str,
    ) -> impl Future<Output = Result<String, HederaProviderError>> + Send;
}

impl HederaFacilitatorOps for HederaChainProvider {
    fn fee_payer_ids(&self) -> Vec<String> {
        Self::fee_payer_ids(self)
    }

    fn verify_payer_signature(
        &self,
        payer: &str,
        transaction_base64: &str,
        network: &str,
    ) -> impl Future<Output = Result<HederaSignatureResult, HederaProviderError>> + Send {
        Self::verify_payer_signature(self, payer, transaction_base64, network)
    }

    fn preflight_transfer(
        &self,
        payer: &str,
        pay_to: &str,
        asset: &str,
        amount: &str,
        network: &str,
    ) -> impl Future<Output = Result<HederaPreflightResult, HederaProviderError>> + Send {
        Self::preflight_transfer(self, payer, pay_to, asset, amount, network)
    }

    fn resolve_account(
        &self,
        account_id_or_alias: &str,
        network: &str,
    ) -> impl Future<Output = Result<HederaAccountResolution, HederaProviderError>> + Send {
        Self::resolve_account(self, account_id_or_alias, network)
    }

    fn sign_and_submit(
        &self,
        transaction_base64: &str,
        fee_payer: &str,
        network: &str,
    ) -> impl Future<Output = Result<String, HederaProviderError>> + Send {
        Self::sign_and_submit(self, transaction_base64, fee_payer, network)
    }
}

/// Facilitator for Hedera exact scheme payments.
pub struct HederaExactFacilitator<P> {
    provider: P,
    alias_policy: AliasPolicy,
    settlement_cache: SettlementCache,
}

impl<P> HederaExactFacilitator<P> {
    /// Creates a facilitator with the default in-memory settlement cache.
    pub fn new(provider: P, alias_policy: AliasPolicy) -> Self {
        Self::with_settlement_cache(provider, alias_policy, SettlementCache::new())
    }

    /// Creates a facilitator with a shared [`SettlementCache`].
    pub const fn with_settlement_cache(
        provider: P,
        alias_policy: AliasPolicy,
        settlement_cache: SettlementCache,
    ) -> Self {
        Self {
            provider,
            alias_policy,
            settlement_cache,
        }
    }
}

impl<P> std::fmt::Debug for HederaExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HederaExactFacilitator")
            .field("alias_policy", &self.alias_policy)
            .finish_non_exhaustive()
    }
}

impl SchemeBuilder<HederaChainProvider> for HederaExact {
    fn build(
        &self,
        provider: HederaChainProvider,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        let parsed = config
            .map(serde_json::from_value::<HederaExactFacilitatorConfig>)
            .transpose()?
            .unwrap_or_default();
        Ok(Box::new(HederaExactFacilitator::new(
            provider,
            parsed.alias_policy,
        )))
    }
}

impl SchemeBuilder<&HederaChainProvider> for HederaExact {
    fn build(
        &self,
        provider: &HederaChainProvider,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        SchemeBuilder::<HederaChainProvider>::build(self, provider.clone(), config)
    }
}

impl<P> Facilitator for HederaExactFacilitator<P>
where
    P: HederaFacilitatorOps + ChainProvider,
{
    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_hedera::exact::verify", skip_all)
    )]
    async fn verify(
        &self,
        request: wire::VerifyRequest,
    ) -> Result<wire::VerifyResponse, FacilitatorError> {
        let json = request.into_json();
        Ok(verify_request_json(&self.provider, self.alias_policy, &json).await)
    }

    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_hedera::exact::settle", skip_all)
    )]
    async fn settle(
        &self,
        request: wire::SettleRequest,
    ) -> Result<wire::SettleResponse, FacilitatorError> {
        let json = request.into_json();
        Ok(settle_request(
            &self.provider,
            self.alias_policy,
            &self.settlement_cache,
            &json,
        )
        .await)
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<wire::SupportedResponse, FacilitatorError>> + Send {
        let chain_id = self.provider.chain_id();
        let extra = self
            .provider
            .fee_payer_ids()
            .into_iter()
            .next()
            .and_then(|fee_payer| {
                serde_json::to_value(HederaExtra {
                    fee_payer: fee_payer.parse().ok()?,
                })
                .ok()
            });
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
            HederaExact.caip_family().into(),
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
