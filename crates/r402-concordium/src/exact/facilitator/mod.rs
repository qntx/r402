//! Facilitator-side payment verification and settlement for Concordium exact.

mod settle;
mod verify;

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use compact_str::CompactString;
use r402_facilitator::{Facilitator, SettlementCache};
use r402_protocol::error::FacilitatorError;
use r402_protocol::network::ChainProvider;
use r402_protocol::payment::{
    SettleRequest, SettleResponse, SupportedPaymentKind, SupportedResponse, V2, VerifyRequest,
    VerifyResponse,
};
use r402_protocol::scheme::SchemeId;
use rand::RngExt;
pub use settle::settle_request;
pub use verify::verify_request_json;

use crate::chain::account::{DEFAULT_FINALIZATION_TIMEOUT_MS, MAX_EXPIRY_OFFSET_SECONDS};
use crate::chain::{ConcordiumChainProvider, ConcordiumNode};
use crate::exact::{ConcordiumExact, ConcordiumExtra, ExactScheme};

/// Facilitator for Concordium exact scheme payments.
pub struct ConcordiumExactFacilitator<N> {
    provider: ConcordiumChainProvider<N>,
    require_finalization: bool,
    finalization_timeout: Duration,
    max_expiry_offset_seconds: u64,
    settlement_cache: SettlementCache,
}

impl<N> ConcordiumExactFacilitator<N> {
    /// Constructs a facilitator. Requires at least one sponsor signer.
    ///
    /// # Errors
    ///
    /// No signers configured.
    pub fn try_new(provider: ConcordiumChainProvider<N>) -> Result<Self, FacilitatorError> {
        if provider.signers().is_empty() {
            return Err(FacilitatorError::aborted(
                "missing_signer",
                "At least one facilitator signer is required",
            ));
        }
        Ok(Self {
            provider,
            require_finalization: true,
            finalization_timeout: Duration::from_millis(DEFAULT_FINALIZATION_TIMEOUT_MS),
            max_expiry_offset_seconds: MAX_EXPIRY_OFFSET_SECONDS,
            settlement_cache: SettlementCache::new(),
        })
    }

    /// Whether settlement requires `finalized` status.
    #[must_use]
    pub const fn with_require_finalization(mut self, require: bool) -> Self {
        self.require_finalization = require;
        self
    }

    /// Finalization wait timeout.
    #[must_use]
    pub const fn with_finalization_timeout(mut self, timeout: Duration) -> Self {
        self.finalization_timeout = timeout;
        self
    }

    /// Maximum expiry offset in seconds (Rule 7).
    #[must_use]
    pub const fn with_max_expiry_offset_seconds(mut self, seconds: u64) -> Self {
        self.max_expiry_offset_seconds = seconds;
        self
    }

    /// Shared settlement cache.
    #[must_use]
    pub fn with_settlement_cache(mut self, cache: SettlementCache) -> Self {
        self.settlement_cache = cache;
        self
    }
}

impl<N> std::fmt::Debug for ConcordiumExactFacilitator<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConcordiumExactFacilitator")
            .field("require_finalization", &self.require_finalization)
            .field("finalization_timeout", &self.finalization_timeout)
            .finish_non_exhaustive()
    }
}

impl<N: ConcordiumNode> Facilitator for ConcordiumExactFacilitator<N> {
    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_concordium::exact::verify", skip_all)
    )]
    async fn verify(&self, request: VerifyRequest) -> Result<VerifyResponse, FacilitatorError> {
        let json = request.into_json();
        let addresses = self.provider.signer_addresses();
        Ok(verify_request_json(
            self.provider.node(),
            &addresses,
            &json,
            self.max_expiry_offset_seconds,
        )
        .await)
    }

    #[cfg_attr(
        feature = "telemetry",
        tracing::instrument(name = "r402_concordium::exact::settle", skip_all)
    )]
    async fn settle(&self, request: SettleRequest) -> Result<SettleResponse, FacilitatorError> {
        let json = request.into_json();
        let addresses = self.provider.signer_addresses();
        Ok(settle_request(
            self.provider.node(),
            self.provider.signers(),
            &addresses,
            &self.settlement_cache,
            &json,
            self.max_expiry_offset_seconds,
            self.require_finalization,
            self.finalization_timeout,
        )
        .await)
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
        let chain_id = self.provider.chain_id();
        let addresses = self.provider.signer_addresses();
        let fee_payer = select_fee_payer(&addresses);
        let extra = fee_payer.and_then(|addr| {
            serde_json::to_value(ConcordiumExtra {
                fee_payer: addr.parse().ok(),
            })
            .ok()
        });
        let kinds = vec![
            SupportedPaymentKind::new(V2.into(), ExactScheme.to_string(), chain_id.to_string())
                .with_optional_extra(extra),
        ];
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::with_capacity(1);
        let _ = signers.insert(
            ConcordiumExact::new().caip_family().into(),
            addresses.into_iter().map(CompactString::from).collect(),
        );
        std::future::ready(Ok(SupportedResponse::new()
            .with_kinds(kinds)
            .with_signers(signers)))
    }
}

fn select_fee_payer(addresses: &[String]) -> Option<&str> {
    if addresses.is_empty() {
        return None;
    }
    let idx = rand::rng().random_range(0..addresses.len());
    addresses.get(idx).map(String::as_str)
}
