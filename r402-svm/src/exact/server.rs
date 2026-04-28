//! Server-side price tag generation for the Solana exact scheme.
//!
//! This module provides functionality for servers to create price tags
//! that clients can use to generate payment authorizations.

use std::sync::Arc;

use r402_core::chain::{ChainId, DeployedTokenAmount};
use r402_core::wire;

use crate::chain::{Address, SolanaTokenDeployment};
use crate::exact::{ExactScheme, SolanaExact, SupportedPaymentKindExtra};

impl SolanaExact {
    /// Creates a price tag for a Solana SPL token payment.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "DeployedTokenAmount is consumed for its fields"
    )]
    pub fn price_tag<A: Into<Address>>(
        pay_to: A,
        asset: DeployedTokenAmount<u64, SolanaTokenDeployment>,
    ) -> wire::PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let requirements = wire::PaymentRequirements::new(
            ExactScheme.to_string().into(),
            chain_id,
            asset.amount.to_string().into(),
            pay_to.into().to_string().into(),
            asset.token.address.to_string().into(),
            300,
        );
        wire::PriceTag {
            requirements,
            enricher: Some(Arc::new(solana_fee_payer_enricher_v2)),
        }
    }
}

/// Enricher function for V2 Solana price tags - adds `fee_payer` to extra field
pub fn solana_fee_payer_enricher_v2(
    price_tag: &mut wire::PriceTag,
    capabilities: &wire::SupportedResponse,
) {
    if price_tag.requirements.extra.is_some() {
        return;
    }

    let extra = capabilities
        .kinds
        .iter()
        .find(|kind| {
            wire::V2 == kind.x402_version
                && kind.scheme.as_str() == ExactScheme.as_ref()
                && kind.network.as_str() == price_tag.requirements.network.to_string()
        })
        .and_then(|kind| kind.extra.as_ref())
        .and_then(|extra| serde_json::from_value::<SupportedPaymentKindExtra>(extra.clone()).ok());

    if let Some(extra) = extra {
        price_tag.requirements.extra = serde_json::to_value(extra).ok();
    }
}
