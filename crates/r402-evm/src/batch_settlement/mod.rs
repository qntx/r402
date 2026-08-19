//! EIP-155 `batch-settlement` scheme (request-path voucher accounting).
//!
//! **Scope (current):** cumulative EOA voucher verify/settle against a
//! pluggable [`ChannelStore`]. Deposit request-path settle is **rejected**
//! until on-chain deposit is implemented; claim/sweep remain operator-side.
//! Treat as experimental for production capital unless the store is durable
//! and channels are funded off-band.
//!
//! Spec: `specs/schemes/batch-settlement/scheme_batch_settlement_evm.md`.

use r402_core::scheme::{BatchSettlementScheme, SchemeId, Sealed};

#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod channel;
#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "facilitator")]
pub mod facilitator;
#[cfg(feature = "server")]
pub mod server;
pub mod store;
pub mod types;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod verify;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod voucher;

#[cfg(any(feature = "client", feature = "facilitator"))]
pub use channel::compute_channel_id;
#[cfg(feature = "client")]
pub use client::{Eip155BatchSettlementClient, sign_voucher_payload};
#[cfg(feature = "facilitator")]
pub use facilitator::Eip155BatchSettlementFacilitator;
pub use store::{ChannelStore, MemoryChannelStore};
pub use types::{
    BATCH_SETTLEMENT_ADDRESS, BATCH_SETTLEMENT_DOMAIN_NAME, BATCH_SETTLEMENT_DOMAIN_VERSION,
    BatchSettlementExtra, BatchSettlementPayload, ChannelConfig, ChannelState,
    ERC3009_DEPOSIT_COLLECTOR_ADDRESS, MAX_WITHDRAW_DELAY, MIN_WITHDRAW_DELAY,
    PERMIT2_DEPOSIT_COLLECTOR_ADDRESS, VoucherFields,
};
#[cfg(feature = "client")]
pub use voucher::sign_voucher;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use voucher::verify_voucher_signature;

/// EIP-155 batch-settlement scheme marker / blueprint.
#[derive(Debug, Clone, Copy, Default)]
pub struct Eip155BatchSettlement;

impl SchemeId for Eip155BatchSettlement {
    fn namespace(&self) -> &'static str {
        "eip155"
    }

    fn scheme(&self) -> &str {
        BatchSettlementScheme.as_ref()
    }
}

impl Sealed for Eip155BatchSettlement {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_id() {
        assert_eq!(Eip155BatchSettlement.namespace(), "eip155");
        assert_eq!(Eip155BatchSettlement.scheme(), "batch-settlement");
    }
}

#[cfg(test)]
mod e2e_tests {
    #[cfg(all(feature = "client", feature = "facilitator"))]
    use alloy_primitives::{Address, B256, U256};
    #[cfg(all(feature = "client", feature = "facilitator"))]
    use alloy_signer_local::PrivateKeySigner;
    #[cfg(all(feature = "client", feature = "facilitator"))]
    use r402_core::scheme::BatchSettlementScheme;
    #[cfg(all(feature = "client", feature = "facilitator"))]
    use r402_core::wire::PaymentPayload;

    #[cfg(all(feature = "client", feature = "facilitator"))]
    use super::channel::compute_channel_id;
    #[cfg(all(feature = "client", feature = "facilitator"))]
    use super::store::{ChannelStore, MemoryChannelStore};
    #[cfg(all(feature = "client", feature = "facilitator"))]
    use super::types::{BatchSettlementExtra, BatchSettlementPayload, ChannelConfig, v2};
    #[cfg(all(feature = "client", feature = "facilitator"))]
    use super::verify::verify_offchain;
    #[cfg(all(feature = "client", feature = "facilitator"))]
    use super::voucher::sign_voucher;
    #[cfg(all(feature = "client", feature = "facilitator"))]
    use crate::chain::{ChecksummedAddress, TokenAmount};

    #[cfg(all(feature = "client", feature = "facilitator"))]
    #[tokio::test]
    async fn voucher_sign_verify_and_charge() {
        let signer = PrivateKeySigner::random();
        let payer = signer.address();
        let cfg = ChannelConfig {
            payer,
            payer_authorizer: payer,
            receiver: Address::repeat_byte(0x22),
            receiver_authorizer: Address::repeat_byte(0x33),
            token: Address::repeat_byte(0x44),
            withdraw_delay: 900,
            salt: B256::ZERO,
        };
        let channel_id = compute_channel_id(&cfg, 8453);
        let charge = U256::from(50_u64);
        let voucher = sign_voucher(&signer, channel_id, charge, 8453)
            .await
            .unwrap();
        let body = BatchSettlementPayload::Voucher {
            channel_config: cfg,
            voucher,
        };
        let extra = BatchSettlementExtra {
            receiver_authorizer: ChecksummedAddress(cfg.receiver_authorizer),
            withdraw_delay: 900,
            name: "USDC".into(),
            version: "2".into(),
            asset_transfer_method: None,
        };
        let requirements = v2::PaymentRequirements::new(
            BatchSettlementScheme,
            "eip155:8453".parse().unwrap(),
            TokenAmount::from(charge),
            ChecksummedAddress(cfg.receiver),
            ChecksummedAddress(cfg.token),
            3600,
        )
        .with_extra(extra);
        let payment: v2::PaymentPayload = PaymentPayload::new(requirements.clone(), body);
        let store = MemoryChannelStore::new();
        let charged = verify_offchain(&payment, &requirements, 8453, &store).unwrap();
        assert_eq!(charged.0, charge);
        assert!(store.try_charge(
            channel_id,
            charged,
            payment.payload.voucher().max_claimable_amount
        ));
        assert_eq!(store.get(&channel_id).charged_cumulative.0, charge);
    }
}
