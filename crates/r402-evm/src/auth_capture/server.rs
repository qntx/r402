//! Server-side price tags for EVM `auth-capture`.

use std::collections::HashMap;
use std::sync::LazyLock;

use alloy_primitives::U256;
use r402_protocol::network::{ChainId, DeployedTokenAmount};
use r402_protocol::payment as wire;
use r402_protocol::scheme::AuthCaptureScheme;
use r402_server::{PaymentFlowConfig, SchemeNetworkServer};

use super::Eip155AuthCapture;
use super::payload::AuthCaptureExtra;
use crate::asset::AssetTransferMethod;
use crate::chain::{ChecksummedAddress, Eip155TokenDeployment};

fn eip155_auth_capture_payment_flows() -> &'static HashMap<String, PaymentFlowConfig> {
    static FLOWS: LazyLock<HashMap<String, PaymentFlowConfig>> = LazyLock::new(|| {
        let row = PaymentFlowConfig::authorization_only();
        HashMap::from([
            ("eip3009".to_owned(), row.clone()),
            ("permit2".to_owned(), row),
        ])
    });
    &FLOWS
}

impl SchemeNetworkServer for Eip155AuthCapture {
    fn scheme(&self) -> &'static str {
        AuthCaptureScheme::VALUE
    }

    fn default_asset_transfer_method(&self) -> &'static str {
        "eip3009"
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        eip155_auth_capture_payment_flows()
    }
}

impl Eip155AuthCapture {
    /// Builds a price tag for auth-capture payments.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "owned DeployedTokenAmount matches Eip155Exact::price_tag"
    )]
    pub fn price_tag<A: Into<ChecksummedAddress>>(
        pay_to: A,
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
        .with_optional_extra(serde_json::to_value(extra).ok());
        wire::PriceTag::new(requirements)
    }

    /// Convenience: fill EIP-712 name/version from the token deployment table.
    #[allow(
        clippy::needless_pass_by_value,
        clippy::too_many_arguments,
        reason = "owned asset matches price_tag; extra fields map 1:1 from deployment metadata"
    )]
    pub fn price_tag_from_deployment<A: Into<ChecksummedAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<U256, Eip155TokenDeployment>,
        capture_authorizer: ChecksummedAddress,
        capture_deadline: u64,
        refund_deadline: u64,
        fee_recipient: ChecksummedAddress,
        min_fee_bps: u16,
        max_fee_bps: u16,
        transfer_method: Option<AssetTransferMethod>,
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
            auto_capture: None,
            asset_transfer_method: transfer_method,
            auth_capture_escrow: None,
        };
        Self::price_tag(pay_to, asset, extra)
    }
}

#[cfg(test)]
mod tests {
    use r402_server::PaymentFlowName;

    use super::*;

    #[test]
    fn payment_flows_are_authorization_only() {
        let scheme = Eip155AuthCapture;
        assert_eq!(scheme.scheme(), "auth-capture");
        assert_eq!(scheme.default_asset_transfer_method(), "eip3009");
        let flows = scheme.payment_flows();
        let expected = PaymentFlowConfig::authorization_only();
        assert_eq!(flows.get("eip3009"), Some(&expected));
        assert_eq!(flows.get("permit2"), Some(&expected));
        assert_eq!(expected.default, PaymentFlowName::Authorization);
    }
}
