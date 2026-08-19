//! EIP-155 `auth-capture` scheme (Base commerce-payments escrow).
//!
//! Client signs ERC-3009 `ReceiveWithAuthorization` or Permit2
//! `PermitTransferFrom` with a nonce derived from the payer-agnostic
//! `PaymentInfo` hash. The facilitator verifies off-chain then calls
//! `AuthCaptureEscrow.authorize` or `.charge`.
//!
//! Spec: `specs/schemes/auth-capture/scheme_auth_capture_evm.md`.

use r402_core::scheme::{AuthCaptureScheme, SchemeId, sealed::Sealed};

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "facilitator")]
pub mod facilitator;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod nonce;
#[cfg(feature = "server")]
pub mod server;
pub mod types;
#[cfg(any(feature = "facilitator", feature = "client"))]
pub mod verify;

#[cfg(feature = "client")]
pub use client::{Eip155AuthCaptureClient, sign_auth_capture};
#[cfg(feature = "facilitator")]
pub use facilitator::Eip155AuthCaptureFacilitator;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use nonce::compute_payer_agnostic_payment_info_hash;
pub use types::{
    AUTH_CAPTURE_CLOCK_SKEW_SECS, AUTH_CAPTURE_ESCROW_ADDRESS, AuthCaptureEip3009Authorization,
    AuthCaptureEip3009Payload, AuthCaptureExtra, AuthCapturePayload,
    AuthCapturePermit2Authorization, AuthCapturePermit2Payload, EIP3009_TOKEN_COLLECTOR_ADDRESS,
    PERMIT2_TOKEN_COLLECTOR_ADDRESS, PaymentInfo,
};

/// EIP-155 auth-capture scheme marker / blueprint.
#[derive(Debug, Clone, Copy, Default)]
pub struct Eip155AuthCapture;

impl SchemeId for Eip155AuthCapture {
    fn namespace(&self) -> &'static str {
        "eip155"
    }

    fn scheme(&self) -> &str {
        AuthCaptureScheme.as_ref()
    }
}

impl Sealed for Eip155AuthCapture {}

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
mod client_verify_tests {
    #[cfg(feature = "client")]
    use alloy_primitives::{Address, U256};
    #[cfg(feature = "client")]
    use alloy_signer_local::PrivateKeySigner;
    #[cfg(feature = "client")]
    use r402_core::scheme::AuthCaptureScheme;
    #[cfg(feature = "client")]
    use r402_core::wire::PaymentPayload;

    #[cfg(feature = "client")]
    use super::client::sign_auth_capture;
    #[cfg(feature = "client")]
    use super::types::v2;
    #[cfg(feature = "client")]
    use super::types::{AuthCaptureExtra, AuthCapturePayload};
    #[cfg(feature = "client")]
    use super::verify::verify_offchain;
    #[cfg(feature = "client")]
    use crate::chain::{ChecksummedAddress, TokenAmount};

    #[cfg(feature = "client")]
    #[tokio::test]
    async fn eip3009_sign_then_verify() {
        let signer = PrivateKeySigner::random();
        let payer = signer.address();
        let now = r402_core::wire::UnixTimestamp::now().as_secs();
        let extra = AuthCaptureExtra {
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
        };
        let asset = Address::repeat_byte(0xAA);
        let pay_to = Address::repeat_byte(0xBB);
        let amount = U256::from(1_000_000_u64);
        let scheme_payload = sign_auth_capture(&signer, 8453, asset, pay_to, amount, 300, &extra)
            .await
            .expect("sign");
        assert!(matches!(scheme_payload, AuthCapturePayload::Eip3009(_)));

        let requirements = v2::PaymentRequirements::new(
            AuthCaptureScheme,
            "eip155:8453".parse().unwrap(),
            TokenAmount::from(amount),
            ChecksummedAddress(pay_to),
            ChecksummedAddress(asset),
            300,
        )
        .with_extra(extra);
        let payment: v2::PaymentPayload = PaymentPayload::new(requirements.clone(), scheme_payload);
        let recovered = verify_offchain(&payment, &requirements, 8453).expect("verify");
        assert_eq!(recovered, payer);
    }

    /// A 64-byte signature must not pass off-chain verification. It clears the
    /// `signature.len() < 64` guard but is not 65 bytes, so signer recovery is
    /// skipped and the payload would otherwise verify with no key ever checked —
    /// letting a caller claim any `from` for free. The sibling batch-settlement
    /// scheme already rejects non-65-byte signatures for this reason.
    #[cfg(feature = "client")]
    #[tokio::test]
    async fn eip3009_rejects_64_byte_signature() {
        let signer = PrivateKeySigner::random();
        let now = r402_core::wire::UnixTimestamp::now().as_secs();
        let extra = AuthCaptureExtra {
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
        };
        let asset = Address::repeat_byte(0xAA);
        let pay_to = Address::repeat_byte(0xBB);
        let amount = U256::from(1_000_000_u64);
        let scheme_payload = sign_auth_capture(&signer, 8453, asset, pay_to, amount, 300, &extra)
            .await
            .expect("sign");

        // Attacker replaces the real 65-byte signature with a 64-byte blob and
        // claims a different `from`. The nonce is payer-agnostic, so it still
        // validates; only the signer gate would catch the swapped payer — and on
        // the unpatched code it is skipped, so verify returns the attacker's
        // chosen address.
        let AuthCapturePayload::Eip3009(mut p) = scheme_payload else {
            unreachable!("sign_auth_capture returns an EIP-3009 payload for this input")
        };
        p.authorization.from = Address::repeat_byte(0xCC);
        p.signature = alloy_primitives::Bytes::from(vec![0u8; 64]);
        let forged = AuthCapturePayload::Eip3009(p);

        let requirements = v2::PaymentRequirements::new(
            AuthCaptureScheme,
            "eip155:8453".parse().unwrap(),
            TokenAmount::from(amount),
            ChecksummedAddress(pay_to),
            ChecksummedAddress(asset),
            300,
        )
        .with_extra(extra);
        let payment: v2::PaymentPayload = PaymentPayload::new(requirements.clone(), forged);

        assert!(
            verify_offchain(&payment, &requirements, 8453).is_err(),
            "a 64-byte signature must not verify off-chain",
        );
    }
}
