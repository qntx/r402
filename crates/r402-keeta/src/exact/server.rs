//! Server-side price tag generation for the Keeta exact scheme.

use std::collections::HashMap;
use std::sync::LazyLock;

use r402_protocol::network::{ChainId, DeployedTokenAmount};
use r402_protocol::payment::{PaymentRequirements, PriceTag};
use r402_protocol::scheme::ExactScheme;
use r402_server::{PaymentFlowConfig, SDK_DEFAULT_ASSET_TRANSFER_METHOD, SchemeNetworkServer};

use crate::chain::{KeetaAddress, KeetaTokenDeployment};
use crate::exact::KeetaExact;

fn keeta_exact_payment_flows() -> &'static HashMap<String, PaymentFlowConfig> {
    static FLOWS: LazyLock<HashMap<String, PaymentFlowConfig>> = LazyLock::new(|| {
        HashMap::from([(
            SDK_DEFAULT_ASSET_TRANSFER_METHOD.to_owned(),
            PaymentFlowConfig::authorization_and_upfront(),
        )])
    });
    &FLOWS
}

impl SchemeNetworkServer for KeetaExact {
    fn scheme(&self) -> &'static str {
        ExactScheme::VALUE
    }

    fn default_asset_transfer_method(&self) -> &'static str {
        SDK_DEFAULT_ASSET_TRANSFER_METHOD
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        keeta_exact_payment_flows()
    }
}

impl KeetaExact {
    /// Creates a price tag for a Keeta token payment.
    ///
    /// Fee payers are facilitator-local and are not surfaced on
    /// `PaymentRequirements.extra`.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "DeployedTokenAmount is consumed for its fields"
    )]
    pub fn price_tag<A: Into<KeetaAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<u128, KeetaTokenDeployment>,
    ) -> PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let requirements = PaymentRequirements::new(
            ExactScheme.to_string().into(),
            chain_id,
            asset.amount.to_string().into(),
            pay_to.into().to_string().into(),
            asset.token.address.to_string().into(),
            300,
        );
        PriceTag::new(requirements)
    }
}

#[cfg(test)]
mod tests {
    use keetanetwork_account::{
        Account, Accountable, GenericAccount, KeyED25519, KeyPairType, Keyable,
    };
    use r402_server::PaymentFlowName;

    use super::*;
    use crate::USDC;
    use crate::chain::KeetaAddress;

    #[test]
    fn price_tag_omits_extra() {
        let account = Account::<KeyED25519>::try_from(Accountable::KeyAndType(
            Keyable::from([9u8; 32]),
            KeyPairType::ED25519,
        ))
        .unwrap();
        let pay_to: KeetaAddress = GenericAccount::Ed25519(account)
            .to_string()
            .parse()
            .unwrap();
        let tag = KeetaExact::price_tag(pay_to, USDC::keeta_testnet().amount(1_000_000u128));
        assert_eq!(tag.requirements.scheme, "exact");
        assert_eq!(tag.requirements.network.to_string(), "keeta:1413829460");
        assert!(tag.requirements.extra.is_none());
        assert_eq!(
            tag.requirements.asset,
            USDC::keeta_testnet().address.to_string()
        );
    }

    #[test]
    fn payment_flows_use_default_authorization_and_upfront() {
        let scheme = KeetaExact;
        assert_eq!(scheme.scheme(), "exact");
        assert_eq!(
            scheme.default_asset_transfer_method(),
            SDK_DEFAULT_ASSET_TRANSFER_METHOD
        );
        let row = scheme
            .payment_flows()
            .get(SDK_DEFAULT_ASSET_TRANSFER_METHOD)
            .expect("default ATM");
        assert_eq!(*row, PaymentFlowConfig::authorization_and_upfront());
        assert_eq!(row.default, PaymentFlowName::Authorization);
        assert!(scheme.dynamic_extra_fields().is_empty());
    }
}
