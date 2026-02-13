//! Server-side price tag generation for the EIP-155 exact scheme.
//!
//! This module provides functionality for servers to create price tags
//! that clients can use to generate payment authorizations.

use alloy_primitives::U256;
use r402::chain::{ChainId, DeployedTokenAmount};
use r402::proto::v2;

use crate::chain::{ChecksummedAddress, Eip155TokenDeployment};
use crate::exact::{AssetTransferMethod, Eip155Exact, ExactScheme, PaymentRequirementsExtra};

impl Eip155Exact {
    /// Creates a price tag for an EVM exact payment.
    ///
    /// Generates a [`v2::PriceTag`] that specifies the payment requirements for a
    /// resource. Uses CAIP-2 chain IDs (e.g., `eip155:8453`) and embeds the
    /// requirements directly in the price tag.
    ///
    /// # Transfer method
    ///
    /// - `None` or `Some(Eip3009)` — EIP-3009 `transferWithAuthorization` (default)
    /// - `Some(Permit2)` — Uniswap Permit2 via `x402Permit2Proxy`
    pub fn price_tag<A: Into<ChecksummedAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<U256, Eip155TokenDeployment>,
        transfer_method: Option<AssetTransferMethod>,
    ) -> v2::PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let extra = PaymentRequirementsExtra::from_deployment(asset.token.eip712, transfer_method);
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
