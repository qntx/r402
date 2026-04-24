//! Server-side price tag generation for the EIP-155 upto scheme.
//!
//! Resource servers use [`Eip155Upto::price_tag`] to declare the **maximum**
//! amount clients may authorise. The actual amount charged at settlement is
//! determined by the server at request time via the
//! [`UptoActualAmount`](../../../../r402_http/server/upto/struct.UptoActualAmount.html)
//! HTTP request extension, and MUST be ≤ the authorised maximum.

use alloy_primitives::U256;
use r402_core::chain::{ChainId, DeployedTokenAmount};
use r402_core::wire;

use crate::chain::{ChecksummedAddress, Eip155TokenDeployment};
use crate::upto::{Eip155Upto, UptoScheme};

impl Eip155Upto {
    /// Creates a price tag for an EVM upto payment.
    ///
    /// The `asset.amount` declares the authorised maximum; clients sign a
    /// Permit2 payload for exactly this value. The resource server MAY
    /// settle for any amount in `[0, max]` at request time.
    ///
    /// Unlike the exact scheme, no `extra` payload is emitted: Permit2's
    /// EIP-712 domain is always `name = "Permit2"` regardless of the token,
    /// and no transfer-method selector is needed (upto is Permit2-exclusive).
    ///
    /// The default `maxTimeoutSeconds` is 300 s; override via
    /// [`wire::PriceTag::with_timeout`].
    #[allow(
        clippy::needless_pass_by_value,
        reason = "mirrors Eip155Exact::price_tag signature for API parity"
    )]
    pub fn price_tag<A: Into<ChecksummedAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<U256, Eip155TokenDeployment>,
    ) -> wire::PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let requirements = wire::PaymentRequirements {
            scheme: UptoScheme.to_string().into(),
            pay_to: pay_to.into().to_string().into(),
            asset: asset.token.address.to_string().into(),
            network: chain_id,
            amount: asset.amount.to_string().into(),
            max_timeout_seconds: 300,
            extra: None,
        };
        wire::PriceTag::new(requirements)
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256};

    use super::*;
    use crate::chain::{Eip155ChainReference, Eip155TokenDeployment};

    #[test]
    fn price_tag_emits_upto_scheme_with_no_extra() {
        let token = Eip155TokenDeployment {
            chain_reference: Eip155ChainReference::new(8453),
            address: Address::ZERO,
            decimals: 6,
            eip712: None,
        };
        let pay_to = ChecksummedAddress::from(Address::repeat_byte(0xCC));
        let tag = Eip155Upto::price_tag(pay_to, token.amount(U256::from(5_000_000_u64)));
        let req = &tag.requirements;
        assert_eq!(req.scheme.as_str(), "upto");
        assert_eq!(req.network.to_string(), "eip155:8453");
        assert_eq!(req.amount.as_str(), "5000000");
        assert_eq!(req.max_timeout_seconds, 300);
        assert!(
            req.extra.is_none(),
            "upto price tag MUST NOT emit an extra payload"
        );
    }
}
