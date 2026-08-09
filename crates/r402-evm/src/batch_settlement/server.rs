//! Server price tags for EVM `batch-settlement`.

use alloy_primitives::U256;
use r402_core::chain::{ChainId, DeployedTokenAmount};
use r402_core::scheme::BatchSettlementScheme;
use r402_core::wire;

use super::Eip155BatchSettlement;
use super::types::BatchSettlementExtra;
use crate::chain::{ChecksummedAddress, Eip155TokenDeployment};
use crate::exact::AssetTransferMethod;

impl Eip155BatchSettlement {
    /// Builds a price tag advertising batch-settlement terms.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "mirrors Eip155Exact::price_tag signature for API parity"
    )]
    pub fn price_tag(
        pay_to: impl Into<ChecksummedAddress>,
        asset: DeployedTokenAmount<U256, Eip155TokenDeployment>,
        extra: BatchSettlementExtra,
    ) -> wire::PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let requirements = wire::PaymentRequirements::new(
            BatchSettlementScheme.to_string().into(),
            chain_id,
            asset.amount.to_string().into(),
            pay_to.into().to_string().into(),
            asset.token.address.to_string().into(),
            3600,
        )
        .with_extra(serde_json::to_value(extra).unwrap_or_else(|_| serde_json::json!({})));
        wire::PriceTag {
            requirements,
            enricher: None,
        }
    }

    /// Convenience constructor from deployment EIP-712 metadata.
    #[must_use]
    pub fn price_tag_from_deployment(
        pay_to: impl Into<ChecksummedAddress>,
        asset: DeployedTokenAmount<U256, Eip155TokenDeployment>,
        receiver_authorizer: ChecksummedAddress,
        withdraw_delay: u64,
        transfer_method: Option<AssetTransferMethod>,
    ) -> wire::PriceTag {
        let (name, version) = asset.token.eip712.as_ref().map_or_else(
            || (String::new(), String::new()),
            |e| (e.name.clone(), e.version.clone()),
        );
        let extra = BatchSettlementExtra {
            receiver_authorizer,
            withdraw_delay,
            name,
            version,
            asset_transfer_method: transfer_method,
        };
        Self::price_tag(pay_to, asset, extra)
    }
}
