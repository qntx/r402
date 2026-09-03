//! Server price tags for EVM `batch-settlement`.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::LazyLock;

use alloy_primitives::U256;
use compact_str::CompactString;
use r402_protocol::network::{ChainId, DeployedTokenAmount};
use r402_protocol::payment::{PaymentRequirements, PriceTag, SupportedPaymentKind};
use r402_protocol::scheme::BatchSettlementScheme;
use r402_server::{FacilitatorSupportError, PaymentFlowConfig, SchemeNetworkServer};

use super::Eip155BatchSettlement;
use super::payload::BatchSettlementExtra;
use crate::asset::AssetTransferMethod;
use crate::chain::{ChecksummedAddress, Eip155TokenDeployment};

fn eip155_batch_settlement_payment_flows() -> &'static HashMap<String, PaymentFlowConfig> {
    static FLOWS: LazyLock<HashMap<String, PaymentFlowConfig>> = LazyLock::new(|| {
        let row = PaymentFlowConfig::authorization_only();
        HashMap::from([
            ("eip3009".to_owned(), row.clone()),
            ("permit2".to_owned(), row),
        ])
    });
    &FLOWS
}

impl SchemeNetworkServer for Eip155BatchSettlement {
    fn scheme(&self) -> &'static str {
        BatchSettlementScheme::VALUE
    }

    fn default_asset_transfer_method(&self) -> &'static str {
        "eip3009"
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        eip155_batch_settlement_payment_flows()
    }

    fn validate_facilitator_support(
        &self,
        network: &ChainId,
        kind: &SupportedPaymentKind,
        facilitator_extensions: &[CompactString],
    ) -> Result<(), FacilitatorSupportError> {
        let _ = (self, facilitator_extensions);
        let scheme = CompactString::from(BatchSettlementScheme::VALUE);
        let Some(raw) = kind
            .extra
            .as_ref()
            .and_then(|extra| extra.get("receiverAuthorizer"))
            .and_then(serde_json::Value::as_str)
        else {
            return Err(FacilitatorSupportError::MissingReceiverAuthorizer {
                scheme,
                network: network.clone(),
            });
        };
        let parsed = ChecksummedAddress::from_str(raw).map_err(|_| {
            FacilitatorSupportError::InvalidReceiverAuthorizer {
                scheme: scheme.clone(),
                network: network.clone(),
            }
        })?;
        if parsed.0.is_zero() {
            return Err(FacilitatorSupportError::ZeroReceiverAuthorizer {
                scheme,
                network: network.clone(),
            });
        }
        Ok(())
    }
}

impl Eip155BatchSettlement {
    /// Builds a price tag advertising batch-settlement terms.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "owned DeployedTokenAmount matches Eip155Exact::price_tag"
    )]
    pub fn price_tag<A: Into<ChecksummedAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<U256, Eip155TokenDeployment>,
        extra: BatchSettlementExtra,
    ) -> PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let requirements = PaymentRequirements::new(
            BatchSettlementScheme.to_string().into(),
            chain_id,
            asset.amount.to_string().into(),
            pay_to.into().to_string().into(),
            asset.token.address.to_string().into(),
            3600,
        )
        .with_optional_extra(serde_json::to_value(extra).ok());
        PriceTag::new(requirements)
    }

    /// Convenience constructor from deployment EIP-712 metadata.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "owned DeployedTokenAmount matches Eip155Exact::price_tag"
    )]
    pub fn price_tag_from_deployment<A: Into<ChecksummedAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<U256, Eip155TokenDeployment>,
        receiver_authorizer: ChecksummedAddress,
        withdraw_delay: u64,
        transfer_method: Option<AssetTransferMethod>,
    ) -> PriceTag {
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

#[cfg(test)]
mod tests {
    use r402_server::PaymentFlowName;

    use super::*;

    fn network() -> ChainId {
        "eip155:1".parse().expect("eip155:1")
    }

    fn kind(extra: Option<serde_json::Value>) -> SupportedPaymentKind {
        SupportedPaymentKind::new(2, "batch-settlement", "eip155:1").with_optional_extra(extra)
    }

    fn support(extra: Option<serde_json::Value>) -> Result<(), FacilitatorSupportError> {
        Eip155BatchSettlement.validate_facilitator_support(&network(), &kind(extra), &[])
    }

    #[test]
    fn payment_flows_are_authorization_only() {
        let scheme = Eip155BatchSettlement;
        assert_eq!(scheme.scheme(), "batch-settlement");
        assert_eq!(scheme.default_asset_transfer_method(), "eip3009");
        let flows = scheme.payment_flows();
        let expected = PaymentFlowConfig::authorization_only();
        assert_eq!(flows.get("eip3009"), Some(&expected));
        assert_eq!(flows.get("permit2"), Some(&expected));
        assert_eq!(expected.default, PaymentFlowName::Authorization);
    }

    #[test]
    fn validate_facilitator_support_missing_receiver_authorizer() {
        assert!(matches!(
            support(None),
            Err(FacilitatorSupportError::MissingReceiverAuthorizer { .. })
        ));
        assert!(matches!(
            support(Some(serde_json::json!({}))),
            Err(FacilitatorSupportError::MissingReceiverAuthorizer { .. })
        ));
        assert!(matches!(
            support(Some(serde_json::json!({ "receiverAuthorizer": null }))),
            Err(FacilitatorSupportError::MissingReceiverAuthorizer { .. })
        ));
        assert!(matches!(
            support(Some(serde_json::json!({ "receiverAuthorizer": 1 }))),
            Err(FacilitatorSupportError::MissingReceiverAuthorizer { .. })
        ));
    }

    #[test]
    fn validate_facilitator_support_invalid_receiver_authorizer() {
        assert!(matches!(
            support(Some(
                serde_json::json!({ "receiverAuthorizer": "not-an-address" })
            )),
            Err(FacilitatorSupportError::InvalidReceiverAuthorizer { .. })
        ));
    }

    #[test]
    fn validate_facilitator_support_zero_receiver_authorizer() {
        assert!(matches!(
            support(Some(serde_json::json!({
                "receiverAuthorizer": "0x0000000000000000000000000000000000000000"
            }))),
            Err(FacilitatorSupportError::ZeroReceiverAuthorizer { .. })
        ));
    }

    #[test]
    fn validate_facilitator_support_nonzero_receiver_authorizer() {
        support(Some(serde_json::json!({
            "receiverAuthorizer": "0x1111111111111111111111111111111111111111"
        })))
        .expect("non-zero checksummed address");
    }
}
