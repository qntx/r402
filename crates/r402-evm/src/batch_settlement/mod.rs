//! EIP-155 `batch-settlement` scheme.
//!
//! Facilitator `/settle` hits the batch-settlement contract for deposit,
//! claim, settle, and refund. Voucher `/verify` is off-chain + [`ChannelStore`];
//! voucher `/settle` is Failure `invalid_batch_settlement_evm_payload_type`.
//!
//! Spec: `specs/schemes/batch-settlement/scheme_batch_settlement_evm.md`.

use r402_protocol::scheme::SchemeId;

#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod channel;
#[cfg(feature = "client")]
pub mod client;
pub mod errors;
#[cfg(feature = "facilitator")]
pub mod facilitator;
pub mod payload;
#[cfg(feature = "server")]
pub mod server;
pub mod store;
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
pub use payload::*;
pub use store::{ChannelStore, MemoryChannelStore};
#[cfg(feature = "client")]
pub use voucher::sign_voucher;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use voucher::verify_voucher_signature;

/// EIP-155 batch-settlement scheme marker.
#[derive(Debug, Clone, Copy)]
pub struct Eip155BatchSettlement;

impl SchemeId for Eip155BatchSettlement {
    fn namespace(&self) -> &'static str {
        "eip155"
    }

    fn scheme(&self) -> &str {
        BatchSettlementScheme.as_ref()
    }
}

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
#[cfg(all(feature = "client", feature = "facilitator"))]
mod e2e_tests {
    use alloy_primitives::{Address, B256, U256};
    use alloy_signer_local::PrivateKeySigner;
    use r402_protocol::payment::PaymentPayload;
    use r402_protocol::scheme::BatchSettlementScheme;

    use super::channel::compute_channel_id;
    use super::payload::{BatchSettlementExtra, BatchSettlementPayload, ChannelConfig, v2};
    use super::store::{ChannelStore, MemoryChannelStore};
    use super::verify::verify_offchain;
    use super::voucher::sign_voucher;
    use crate::chain::{ChecksummedAddress, TokenAmount};

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
        let channel_id = compute_channel_id(&cfg, 8453).expect("id");
        let charge = U256::from(50_u64);
        let voucher = sign_voucher(&signer, channel_id, charge, 8453)
            .await
            .expect("sign");
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
            "eip155:8453".parse().expect("chain"),
            TokenAmount::from(charge),
            ChecksummedAddress(cfg.receiver),
            ChecksummedAddress(cfg.token),
            3600,
        )
        .with_extra(extra);
        let payment: v2::PaymentPayload = PaymentPayload::new(requirements.clone(), body);
        let store = MemoryChannelStore::new();
        let charged = verify_offchain(&payment, &requirements, 8453, &store).expect("verify");
        assert_eq!(charged.0, charge);
        assert!(
            store.try_charge(
                channel_id,
                charged,
                payment
                    .payload
                    .voucher()
                    .expect("voucher payload")
                    .max_claimable_amount
            )
        );
        assert_eq!(store.get(&channel_id).charged_cumulative.0, charge);
    }

    #[tokio::test]
    async fn extra_900_with_config_uint40_overflow_is_invalid_format() {
        let cfg = ChannelConfig {
            payer: Address::repeat_byte(0x11),
            payer_authorizer: Address::repeat_byte(0x11),
            receiver: Address::repeat_byte(0x22),
            receiver_authorizer: Address::repeat_byte(0x33),
            token: Address::repeat_byte(0x44),
            withdraw_delay: 1 << 40,
            salt: B256::ZERO,
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
            "eip155:8453".parse().expect("chain"),
            TokenAmount::from(U256::from(50_u64)),
            ChecksummedAddress(cfg.receiver),
            ChecksummedAddress(cfg.token),
            3600,
        )
        .with_extra(extra);
        let body = BatchSettlementPayload::Voucher {
            channel_config: cfg,
            voucher: super::payload::VoucherFields {
                channel_id: B256::ZERO,
                max_claimable_amount: TokenAmount::from(U256::from(50_u64)),
                signature: alloy_primitives::Bytes::from(vec![0u8; 65]),
            },
        };
        let payment: v2::PaymentPayload = PaymentPayload::new(requirements.clone(), body);
        let store = MemoryChannelStore::new();
        assert!(
            matches!(
                verify_offchain(&payment, &requirements, 8453, &store),
                Err(r402_protocol::error::VerificationError::InvalidFormat(_))
            ),
            "overflow withdrawDelay must be InvalidFormat, not panic"
        );
    }

    #[tokio::test]
    async fn refund_on_empty_store_is_invalid_format() {
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
        let channel_id = compute_channel_id(&cfg, 8453).expect("id");
        let voucher = sign_voucher(&signer, channel_id, U256::ZERO, 8453)
            .await
            .expect("sign");
        let body = BatchSettlementPayload::Refund {
            channel_config: cfg,
            voucher,
            amount: None,
            refund_nonce: None,
            claims: None,
            refund_authorizer_signature: None,
            claim_authorizer_signature: None,
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
            "eip155:8453".parse().expect("chain"),
            TokenAmount::from(U256::ZERO),
            ChecksummedAddress(cfg.receiver),
            ChecksummedAddress(cfg.token),
            3600,
        )
        .with_extra(extra);
        let payment: v2::PaymentPayload = PaymentPayload::new(requirements.clone(), body);
        let store = MemoryChannelStore::new();
        assert!(
            matches!(
                verify_offchain(&payment, &requirements, 8453, &store),
                Err(r402_protocol::error::VerificationError::InvalidFormat(_))
            ),
            "refund must not verify as paid"
        );
    }
}
