//! Facilitator-side payment verification and settlement for the Algorand exact scheme.

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

use crate::chain::AlgorandChainProvider;
use crate::chain::codec::encode_signed;
use crate::chain::rpc::wait_for_confirmation;
use crate::exact::error::invalid;
use crate::exact::{AlgorandExact, AlgorandExtra, DEFAULT_WAIT_ROUNDS, ExactScheme};

/// Facilitator for Algorand exact scheme payments.
pub struct AlgorandExactFacilitator<P> {
    provider: P,
    wait_rounds: u32,
    settlement_cache: SettlementCache,
}

impl<P> AlgorandExactFacilitator<P> {
    /// Constructs a facilitator from a chain provider.
    ///
    /// Uses [`DEFAULT_WAIT_ROUNDS`] and a default [`SettlementCache`].
    ///
    /// # Errors
    ///
    /// Currently infallible. [`Result`] so `try_new(provider)?` compiles.
    #[allow(
        clippy::unnecessary_wraps,
        reason = "Result so try_new(provider)? compiles"
    )]
    pub fn try_new(provider: P) -> Result<Self, FacilitatorError> {
        Ok(Self::new(provider, DEFAULT_WAIT_ROUNDS))
    }

    /// Creates a facilitator with the default in-memory settlement cache.
    #[must_use]
    pub fn new(provider: P, wait_rounds: u32) -> Self {
        Self::with_settlement_cache(provider, wait_rounds, SettlementCache::new())
    }

    /// Creates a facilitator with a shared [`SettlementCache`].
    #[must_use]
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

    /// Sets the number of rounds to wait for confirmation after broadcast.
    #[must_use]
    pub const fn with_wait_rounds(mut self, wait_rounds: u32) -> Self {
        self.wait_rounds = wait_rounds;
        self
    }
}

impl<P> std::fmt::Debug for AlgorandExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlgorandExactFacilitator")
            .field("wait_rounds", &self.wait_rounds)
            .finish_non_exhaustive()
    }
}

impl Facilitator for AlgorandExactFacilitator<AlgorandChainProvider> {
    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_avm::exact::verify", skip_all)
    )]
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
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
        tracing::instrument(name = "r402_avm::exact::settle", skip_all)
    )]
    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
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
    ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
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
            SupportedPaymentKind::new(V2.into(), ExactScheme.to_string(), chain_id.to_string())
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
        std::future::ready(Ok(SupportedResponse::new()
            .with_kinds(kinds)
            .with_signers(signers)))
    }
}
