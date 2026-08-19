//! Server-side price tag generation for the Keeta exact scheme.

use r402_core::chain::{ChainId, DeployedTokenAmount};
use r402_core::wire;

use crate::chain::{KeetaAddress, KeetaTokenDeployment};
use crate::exact::{ExactScheme, KeetaExact};

impl KeetaExact {
    /// Creates a price tag for a Keeta token payment.
    ///
    /// The enricher is `None`: fee payers are facilitator-local and are not
    /// surfaced on `PaymentRequirements.extra`.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "DeployedTokenAmount is consumed for its fields"
    )]
    pub fn price_tag<A: Into<KeetaAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<u128, KeetaTokenDeployment>,
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
            enricher: None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;
    use crate::USDC;

    #[test]
    fn price_tag_omits_extra_and_enricher() {
        use keetanetwork_account::{
            Account, Accountable, GenericAccount, KeyED25519, KeyPairType, Keyable,
        };
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
        assert!(tag.enricher.is_none());
        assert_eq!(
            tag.requirements.asset,
            USDC::keeta_testnet().address.to_string()
        );
    }
}
