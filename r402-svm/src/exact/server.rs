//! Server-side price tag generation for the Solana exact scheme.
//!
//! This module provides functionality for servers to create V1 and V2 price tags
//! that clients can use to generate payment authorizations.

use r402::chain::{ChainId, DeployedTokenAmount};
use r402::proto;
use r402::proto::{v1, v2};
use std::sync::Arc;

use crate::chain::{Address, SolanaTokenDeployment};
use crate::exact::{ExactScheme, SupportedPaymentKindExtra, V1SolanaExact, V2SolanaExact};

impl V1SolanaExact {
    /// Creates a V1 price tag for a Solana SPL token payment.
    ///
    /// # Panics
    ///
    /// Panics if the chain ID has no known network name.
    #[allow(clippy::panic, clippy::needless_pass_by_value)]
    pub fn price_tag<A: Into<Address>>(
        pay_to: A,
        asset: DeployedTokenAmount<u64, SolanaTokenDeployment>,
    ) -> v1::PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let network = chain_id
            .as_network_name()
            .unwrap_or_else(|| panic!("Can not get network name for chain id {chain_id}"));
        v1::PriceTag {
            scheme: ExactScheme.to_string(),
            pay_to: pay_to.into().to_string(),
            asset: asset.token.address.to_string(),
            network: network.to_string(),
            amount: asset.amount.to_string(),
            max_timeout_seconds: 300,
            extra: None,
            enricher: Some(Arc::new(solana_fee_payer_enricher)),
        }
    }
}

/// Enricher function for V1 Solana price tags - adds `fee_payer` to extra field
pub fn solana_fee_payer_enricher(
    price_tag: &mut v1::PriceTag,
    capabilities: &proto::SupportedResponse,
) {
    if price_tag.extra.is_some() {
        return;
    }

    let extra = capabilities
        .kinds
        .iter()
        .find(|kind| {
            v1::X402Version1 == kind.x402_version
                && kind.scheme == ExactScheme.to_string()
                && kind.network == price_tag.network
        })
        .and_then(|kind| kind.extra.as_ref())
        .and_then(|extra| serde_json::from_value::<SupportedPaymentKindExtra>(extra.clone()).ok());

    if let Some(extra) = extra {
        price_tag.extra = serde_json::to_value(extra).ok();
    }
}

impl V2SolanaExact {
    /// Creates a V2 price tag for a Solana SPL token payment.
    #[allow(clippy::needless_pass_by_value)]
    pub fn price_tag<A: Into<Address>>(
        pay_to: A,
        asset: DeployedTokenAmount<u64, SolanaTokenDeployment>,
    ) -> v2::PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let requirements = v2::PaymentRequirements {
            scheme: ExactScheme.to_string(),
            pay_to: pay_to.into().to_string(),
            asset: asset.token.address.to_string(),
            network: chain_id,
            amount: asset.amount.to_string(),
            max_timeout_seconds: 300,
            extra: None,
        };
        v2::PriceTag {
            requirements,
            enricher: Some(Arc::new(solana_fee_payer_enricher_v2)),
        }
    }
}

/// Enricher function for V2 Solana price tags - adds `fee_payer` to extra field
pub fn solana_fee_payer_enricher_v2(
    price_tag: &mut v2::PriceTag,
    capabilities: &proto::SupportedResponse,
) {
    if price_tag.requirements.extra.is_some() {
        return;
    }

    let extra = capabilities
        .kinds
        .iter()
        .find(|kind| {
            v2::X402Version2 == kind.x402_version
                && kind.scheme == ExactScheme.to_string()
                && kind.network == price_tag.requirements.network.to_string()
        })
        .and_then(|kind| kind.extra.as_ref())
        .and_then(|extra| serde_json::from_value::<SupportedPaymentKindExtra>(extra.clone()).ok());

    if let Some(extra) = extra {
        price_tag.requirements.extra = serde_json::to_value(extra).ok();
    }
}
