//! Server-side price tags for EVM `auth-capture`.

use alloy_primitives::U256;
use r402_core::chain::{ChainId, DeployedTokenAmount};
use r402_core::scheme::AuthCaptureScheme;
use r402_core::wire;

use super::Eip155AuthCapture;
use super::types::AuthCaptureExtra;
use crate::chain::{ChecksummedAddress, Eip155TokenDeployment};
use crate::exact::AssetTransferMethod;

impl Eip155AuthCapture {
    /// Builds a price tag for auth-capture payments.
    ///
    /// # Panics
    ///
    /// Never panics for valid inputs; fee deadlines are absolute Unix times
    /// supplied by the caller.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "mirrors Eip155Exact::price_tag signature for API parity"
    )]
    pub fn price_tag(
        pay_to: impl Into<ChecksummedAddress>,
        asset: DeployedTokenAmount<U256, Eip155TokenDeployment>,
        extra: AuthCaptureExtra,
    ) -> wire::PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let requirements = wire::PaymentRequirements::new(
            AuthCaptureScheme.to_string().into(),
            chain_id,
            asset.amount.to_string().into(),
            pay_to.into().to_string().into(),
            asset.token.address.to_string().into(),
            300,
        )
        .with_extra(serde_json::to_value(extra).unwrap_or_else(|_| serde_json::json!({})));
        wire::PriceTag {
            requirements,
            enricher: None,
        }
    }

    /// Convenience: fill EIP-712 name/version from the token deployment table.
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "maps 1:1 to AuthCaptureExtra fields from deployment metadata"
    )]
    pub fn price_tag_from_deployment(
        pay_to: impl Into<ChecksummedAddress>,
        asset: DeployedTokenAmount<U256, Eip155TokenDeployment>,
        capture_authorizer: ChecksummedAddress,
        capture_deadline: u64,
        refund_deadline: u64,
        fee_recipient: ChecksummedAddress,
        min_fee_bps: u16,
        max_fee_bps: u16,
        transfer_method: Option<AssetTransferMethod>,
        auto_capture: bool,
    ) -> wire::PriceTag {
        let (name, version) = asset.token.eip712.as_ref().map_or_else(
            || (String::new(), String::new()),
            |e| (e.name.clone(), e.version.clone()),
        );
        let extra = AuthCaptureExtra {
            name,
            version,
            capture_authorizer,
            capture_deadline,
            refund_deadline,
            fee_recipient,
            min_fee_bps,
            max_fee_bps,
            auto_capture: Some(auto_capture),
            asset_transfer_method: transfer_method,
        };
        Self::price_tag(pay_to, asset, extra)
    }
}
