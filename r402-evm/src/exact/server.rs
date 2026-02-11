//! Server-side price tag generation for the EIP-155 exact scheme.
//!
//! This module provides functionality for servers to create price tags
//! that clients can use to generate payment authorizations.

use alloy_primitives::U256;
use r402::chain::{ChainId, DeployedTokenAmount};
use r402::proto::v2;

use crate::chain::{ChecksummedAddress, Eip155TokenDeployment};
use crate::exact::{Eip155Exact, ExactScheme};

impl Eip155Exact {
    /// Creates a price tag for an ERC-3009 payment on an EVM chain.
    ///
    /// This function generates a price tag that specifies the payment requirements
    /// for a resource. Uses CAIP-2 chain IDs (e.g., `eip155:8453`) and embeds
    /// the requirements directly in the price tag.
    ///
    /// # Parameters
    ///
    /// - `pay_to`: The recipient address (can be any type convertible to [`ChecksummedAddress`])
    /// - `asset`: The token deployment and amount required
    ///
    /// # Returns
    ///
    /// A [`v2::PriceTag`] that can be included in a `PaymentRequired` response.
    pub fn price_tag<A: Into<ChecksummedAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<U256, Eip155TokenDeployment>,
    ) -> v2::PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let extra = asset
            .token
            .eip712
            .and_then(|eip712| serde_json::to_value(&eip712).ok());
        let requirements = v2::PaymentRequirements {
            scheme: ExactScheme.to_string(),
            pay_to: pay_to.into().to_string(),
            asset: asset.token.address.to_string(),
            network: chain_id,
            amount: asset.amount.to_string(),
            max_timeout_seconds: 300,
            extra,
        };
        v2::PriceTag {
            requirements,
            enricher: None,
        }
    }
}
