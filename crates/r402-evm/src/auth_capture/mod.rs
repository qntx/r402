//! EIP-155 `auth-capture` scheme (Base commerce-payments escrow).
//!
//! Client signs ERC-3009 `ReceiveWithAuthorization` or Permit2
//! `PermitTransferFrom` with a nonce derived from the payer-agnostic
//! `PaymentInfo` hash. The facilitator verifies off-chain then calls
//! `AuthCaptureEscrow.authorize`.
//!
//! Spec: `specs/schemes/auth-capture/scheme_auth_capture_evm.md`.

use r402_protocol::scheme::SchemeId;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "facilitator")]
pub mod facilitator;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod nonce;
pub mod payload;
#[cfg(feature = "server")]
pub mod server;
#[cfg(any(feature = "facilitator", feature = "client"))]
pub mod verify;

#[cfg(feature = "client")]
pub use client::{Eip155AuthCaptureClient, sign_auth_capture};
#[cfg(feature = "facilitator")]
pub use facilitator::Eip155AuthCaptureFacilitator;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use nonce::compute_payer_agnostic_payment_info_hash;
pub use payload::*;

/// EIP-155 auth-capture scheme marker.
#[derive(Debug, Clone, Copy)]
pub struct Eip155AuthCapture;

impl SchemeId for Eip155AuthCapture {
    fn namespace(&self) -> &'static str {
        "eip155"
    }

    fn scheme(&self) -> &str {
        AuthCaptureScheme.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_id() {
        assert_eq!(Eip155AuthCapture.namespace(), "eip155");
        assert_eq!(Eip155AuthCapture.scheme(), "auth-capture");
    }
}

#[cfg(test)]
#[cfg(feature = "client")]
mod client_verify_tests {
    use alloy_primitives::{Address, U256};
    use alloy_signer_local::PrivateKeySigner;
    use r402_protocol::error::VerificationError;
    use r402_protocol::payment::PaymentPayload;
    use r402_protocol::scheme::AuthCaptureScheme;

    use super::client::sign_auth_capture;
    use super::payload::{AuthCaptureExtra, AuthCapturePayload, v2};
    use super::verify::verify_offchain;
    use crate::chain::{ChecksummedAddress, TokenAmount};

    fn extra(now: u64) -> AuthCaptureExtra {
        AuthCaptureExtra {
            name: "USD Coin".into(),
            version: "2".into(),
            capture_authorizer: ChecksummedAddress(Address::repeat_byte(0x11)),
            capture_deadline: now + 3600,
            refund_deadline: now + 86400,
            fee_recipient: ChecksummedAddress(Address::repeat_byte(0x22)),
            min_fee_bps: 0,
            max_fee_bps: 1000,
            auto_capture: Some(false),
            asset_transfer_method: None,
        }
    }

    fn requirements(
        extra: AuthCaptureExtra,
        amount: U256,
        pay_to: Address,
        asset: Address,
    ) -> v2::PaymentRequirements {
        v2::PaymentRequirements::new(
            AuthCaptureScheme,
            "eip155:8453".parse().expect("chain"),
            TokenAmount::from(amount),
            ChecksummedAddress(pay_to),
            ChecksummedAddress(asset),
            300,
        )
        .with_extra(extra)
    }

    #[tokio::test]
    async fn eip3009_sign_then_verify() {
        let signer = PrivateKeySigner::random();
        let payer = signer.address();
        let now = r402_protocol::payment::UnixTimestamp::now().as_secs();
        let extra = extra(now);
        let asset = Address::repeat_byte(0xAA);
        let pay_to = Address::repeat_byte(0xBB);
        let amount = U256::from(1_000_000_u64);
        let scheme_payload = sign_auth_capture(&signer, 8453, asset, pay_to, amount, 300, &extra)
            .await
            .expect("sign");
        assert!(matches!(scheme_payload, AuthCapturePayload::Eip3009(_)));

        let requirements = requirements(extra, amount, pay_to, asset);
        let payment: v2::PaymentPayload = PaymentPayload::new(requirements.clone(), scheme_payload);
        let recovered = verify_offchain(&payment, &requirements, 8453).expect("verify");
        assert_eq!(recovered, payer);
    }

    #[tokio::test]
    async fn eip3009_rejects_64_byte_signature() {
        let signer = PrivateKeySigner::random();
        let now = r402_protocol::payment::UnixTimestamp::now().as_secs();
        let extra = extra(now);
        let asset = Address::repeat_byte(0xAA);
        let pay_to = Address::repeat_byte(0xBB);
        let amount = U256::from(1_000_000_u64);
        let scheme_payload = sign_auth_capture(&signer, 8453, asset, pay_to, amount, 300, &extra)
            .await
            .expect("sign");

        let mut p = match scheme_payload {
            AuthCapturePayload::Eip3009(p) => p,
            AuthCapturePayload::Permit2(_) => {
                panic!("sign_auth_capture with default extra is Eip3009")
            }
        };
        p.authorization.from = Address::repeat_byte(0xCC);
        p.signature = alloy_primitives::Bytes::from(vec![0u8; 64]);
        let forged = AuthCapturePayload::Eip3009(p);

        let requirements = requirements(extra, amount, pay_to, asset);
        let payment: v2::PaymentPayload = PaymentPayload::new(requirements.clone(), forged);

        assert!(
            matches!(
                verify_offchain(&payment, &requirements, 8453),
                Err(VerificationError::InvalidSignature(_))
            ),
            "a 64-byte signature must not verify off-chain",
        );
    }

    #[tokio::test]
    async fn eip3009_rejects_66_byte_signature() {
        let signer = PrivateKeySigner::random();
        let now = r402_protocol::payment::UnixTimestamp::now().as_secs();
        let extra = extra(now);
        let asset = Address::repeat_byte(0xAA);
        let pay_to = Address::repeat_byte(0xBB);
        let amount = U256::from(1_000_000_u64);
        let scheme_payload = sign_auth_capture(&signer, 8453, asset, pay_to, amount, 300, &extra)
            .await
            .expect("sign");

        let mut p = match scheme_payload {
            AuthCapturePayload::Eip3009(p) => p,
            AuthCapturePayload::Permit2(_) => {
                panic!("sign_auth_capture with default extra is Eip3009")
            }
        };
        p.authorization.from = Address::repeat_byte(0xCC);
        p.signature = alloy_primitives::Bytes::from(vec![0u8; 66]);
        let forged = AuthCapturePayload::Eip3009(p);

        let requirements = requirements(extra, amount, pay_to, asset);
        let payment: v2::PaymentPayload = PaymentPayload::new(requirements.clone(), forged);

        assert!(
            matches!(
                verify_offchain(&payment, &requirements, 8453),
                Err(VerificationError::InvalidSignature(_))
            ),
            "a 66-byte signature must not verify off-chain",
        );
    }

    #[tokio::test]
    async fn eip3009_rejects_trailing_byte_on_valid_signature() {
        let signer = PrivateKeySigner::random();
        let now = r402_protocol::payment::UnixTimestamp::now().as_secs();
        let extra = extra(now);
        let asset = Address::repeat_byte(0xAA);
        let pay_to = Address::repeat_byte(0xBB);
        let amount = U256::from(1_000_000_u64);
        let scheme_payload = sign_auth_capture(&signer, 8453, asset, pay_to, amount, 300, &extra)
            .await
            .expect("sign");

        let mut p = match scheme_payload {
            AuthCapturePayload::Eip3009(p) => p,
            AuthCapturePayload::Permit2(_) => {
                panic!("sign_auth_capture with default extra is Eip3009")
            }
        };
        let mut longer = p.signature.to_vec();
        longer.push(0);
        p.signature = alloy_primitives::Bytes::from(longer);
        let forged = AuthCapturePayload::Eip3009(p);

        let requirements = requirements(extra, amount, pay_to, asset);
        let payment: v2::PaymentPayload = PaymentPayload::new(requirements.clone(), forged);

        assert!(
            matches!(
                verify_offchain(&payment, &requirements, 8453),
                Err(VerificationError::InvalidSignature(_))
            ),
            "a 65-byte signature with a trailing byte must not verify off-chain",
        );
    }

    #[tokio::test]
    async fn permit2_sign_then_verify() {
        use crate::asset::AssetTransferMethod;

        let signer = PrivateKeySigner::random();
        let payer = signer.address();
        let now = r402_protocol::payment::UnixTimestamp::now().as_secs();
        let mut extra = extra(now);
        extra.asset_transfer_method = Some(AssetTransferMethod::Permit2);
        let asset = Address::repeat_byte(0xAA);
        let pay_to = Address::repeat_byte(0xBB);
        let amount = U256::from(1_000_000_u64);
        let scheme_payload = sign_auth_capture(&signer, 8453, asset, pay_to, amount, 300, &extra)
            .await
            .expect("sign");
        assert!(matches!(scheme_payload, AuthCapturePayload::Permit2(_)));

        let requirements = requirements(extra, amount, pay_to, asset);
        let payment: v2::PaymentPayload = PaymentPayload::new(requirements.clone(), scheme_payload);
        let recovered = verify_offchain(&payment, &requirements, 8453).expect("verify");
        assert_eq!(recovered, payer);
    }

    #[tokio::test]
    async fn auto_capture_true_is_invalid_format() {
        let signer = PrivateKeySigner::random();
        let now = r402_protocol::payment::UnixTimestamp::now().as_secs();
        let mut extra = extra(now);
        extra.auto_capture = Some(true);
        let asset = Address::repeat_byte(0xAA);
        let pay_to = Address::repeat_byte(0xBB);
        let amount = U256::from(1_000_000_u64);
        let scheme_payload = sign_auth_capture(&signer, 8453, asset, pay_to, amount, 300, &extra)
            .await
            .expect("sign");
        let requirements = requirements(extra, amount, pay_to, asset);
        let payment: v2::PaymentPayload = PaymentPayload::new(requirements.clone(), scheme_payload);
        assert!(
            matches!(
                verify_offchain(&payment, &requirements, 8453),
                Err(VerificationError::InvalidFormat(_))
            ),
            "autoCapture true is an unsupported payment flow"
        );
    }
}
