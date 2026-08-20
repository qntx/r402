//! Facilitator-side payment verification and settlement for the Algorand exact scheme.

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

use crate::DEFAULT_WAIT_ROUNDS;
use crate::chain::AlgorandChainProvider;
use crate::chain::codec::encode_signed;
use crate::chain::rpc::wait_for_confirmation;
use crate::exact::error::invalid;
use crate::exact::{AlgorandExact, AlgorandExtra, ExactScheme};

/// Optional JSON config for [`AlgorandExact`] [`SchemeBuilder`].
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlgorandExactFacilitatorConfig {
    /// Rounds to wait for confirmation after broadcast.
    ///
    /// When omitted, [`crate::DEFAULT_WAIT_ROUNDS`] is used.
    #[serde(default)]
    pub wait_rounds: Option<u32>,
}

/// Facilitator for Algorand exact scheme payments.
pub struct AlgorandExactFacilitator<P> {
    provider: P,
    wait_rounds: u32,
    settlement_cache: SettlementCache,
}

impl<P> AlgorandExactFacilitator<P> {
    /// Creates a facilitator with the default in-memory settlement cache.
    pub fn new(provider: P, wait_rounds: u32) -> Self {
        Self::with_settlement_cache(provider, wait_rounds, SettlementCache::new())
    }

    /// Creates a facilitator with a shared [`SettlementCache`].
    pub const fn with_settlement_cache(
        provider: P,
        wait_rounds: u32,
        settlement_cache: SettlementCache,
    ) -> Self {
        Self {
            provider,
            wait_rounds,
            settlement_cache,
        }
    }
}

impl<P> std::fmt::Debug for AlgorandExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlgorandExactFacilitator")
            .field("wait_rounds", &self.wait_rounds)
            .finish_non_exhaustive()
    }
}

impl SchemeBuilder<AlgorandChainProvider> for AlgorandExact {
    fn build(
        &self,
        provider: AlgorandChainProvider,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        let parsed = config
            .map(serde_json::from_value::<AlgorandExactFacilitatorConfig>)
            .transpose()?
            .unwrap_or_default();
        let wait_rounds = parsed.wait_rounds.unwrap_or(DEFAULT_WAIT_ROUNDS);
        Ok(Box::new(AlgorandExactFacilitator::new(
            provider,
            wait_rounds,
        )))
    }
}

impl SchemeBuilder<&AlgorandChainProvider> for AlgorandExact {
    fn build(
        &self,
        provider: &AlgorandChainProvider,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        SchemeBuilder::<AlgorandChainProvider>::build(self, provider.clone(), config)
    }
}

impl Facilitator for AlgorandExactFacilitator<AlgorandChainProvider> {
    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_algorand::exact::verify", skip_all)
    )]
    async fn verify(
        &self,
        request: wire::VerifyRequest,
    ) -> Result<wire::VerifyResponse, FacilitatorError> {
        let json = request.into_json();
        let addresses = self.provider.signer_addresses();
        let provider = &self.provider;
        let response = verify_request_json(provider, &addresses, &json, |txn, sender| {
            let signer = provider.signer_for(txn.sender).ok_or_else(|| {
                invalid("invalid_exact_avm_invalid_fee_payer")
                    .with_message(format!("no signer for {sender}"))
            })?;
            Ok(encode_signed(&signer.sign(txn)))
        })
        .await;
        Ok(response)
    }

    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_algorand::exact::settle", skip_all)
    )]
    async fn settle(
        &self,
        request: wire::SettleRequest,
    ) -> Result<wire::SettleResponse, FacilitatorError> {
        let json = request.into_json();
        let addresses = self.provider.signer_addresses();
        let provider = self.provider.clone();
        let wait_rounds = self.wait_rounds;
        let response = settle_request(
            &self.provider,
            &addresses,
            &self.settlement_cache,
            wait_rounds,
            &json,
            |txn, sender| {
                let signer = provider.signer_for(txn.sender).ok_or_else(|| {
                    invalid("invalid_exact_avm_invalid_fee_payer")
                        .with_message(format!("no signer for {sender}"))
                })?;
                Ok(encode_signed(&signer.sign(txn)))
            },
            |txid, rounds| {
                let provider = provider.clone();
                async move { wait_for_confirmation(&provider, &txid, rounds).await }
            },
        )
        .await;
        Ok(response)
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<wire::SupportedResponse, FacilitatorError>> + Send {
        let chain_id = self.provider.chain_id();
        let extra = self
            .provider
            .signer_addresses()
            .into_iter()
            .next()
            .and_then(|addr| addr.parse().ok())
            .map(|fee_payer| AlgorandExtra {
                fee_payer: Some(fee_payer),
            })
            .and_then(|extra| serde_json::to_value(extra).ok());
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
            AlgorandExact.caip_family().into(),
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
