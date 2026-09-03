//! Wire types and constants for EVM `auth-capture`.
//!
//! Addresses and field names match
//! `specs/schemes/auth-capture/scheme_auth_capture_evm.md`.

use alloy_primitives::{Address, B256, Bytes, address};
use r402_protocol::payment::UnixTimestamp;
pub use r402_protocol::scheme::AuthCaptureScheme;
use serde::{Deserialize, Serialize};

use crate::asset::AssetTransferMethod;
use crate::chain::{ChecksummedAddress, TokenAmount};

/// Canonical `AuthCaptureEscrow` (base/commerce-payments), same on all chains.
pub const AUTH_CAPTURE_ESCROW_ADDRESS: Address =
    address!("0xBdEA0D1bcC5966192B070Fdf62aB4EF5b4420cff");

/// EIP-3009 token collector used as `authorization.to`.
pub const EIP3009_TOKEN_COLLECTOR_ADDRESS: Address =
    address!("0x0E3dF9510de65469C4518D7843919c0b8C7A7757");

/// Permit2 token collector used as permit `spender`.
pub const PERMIT2_TOKEN_COLLECTOR_ADDRESS: Address =
    address!("0x992476B9Ee81d52a5BdA0622C333938D0Af0aB26");

/// Clock-skew buffer (seconds) for deadline checks — matches foundation SDKs.
pub const AUTH_CAPTURE_CLOCK_SKEW_SECS: u64 = 6;

/// Server `PaymentRequirements.extra` for auth-capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthCaptureExtra {
    /// EIP-712 token domain name.
    pub name: String,
    /// EIP-712 token domain version.
    pub version: String,
    /// On-chain `PaymentInfo.operator` (capture authorizer).
    pub capture_authorizer: ChecksummedAddress,
    /// Absolute Unix seconds — capture must occur before this.
    pub capture_deadline: u64,
    /// Absolute Unix seconds — refunds allowed until this.
    pub refund_deadline: u64,
    /// Fee recipient (`PaymentInfo.feeReceiver`).
    pub fee_recipient: ChecksummedAddress,
    /// Minimum fee in basis points.
    pub min_fee_bps: u16,
    /// Maximum fee in basis points.
    pub max_fee_bps: u16,
    /// When `true`, facilitator calls `charge()`; otherwise `authorize()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_capture: Option<bool>,
    /// Transfer method; default eip3009 when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_transfer_method: Option<AssetTransferMethod>,
}

impl AuthCaptureExtra {
    /// Effective transfer method (default EIP-3009).
    #[must_use]
    pub const fn transfer_method(&self) -> AssetTransferMethod {
        match self.asset_transfer_method {
            Some(m) => m,
            None => AssetTransferMethod::Eip3009,
        }
    }

    /// `extra.autoCapture == true`. Verify rejects this: v1.0 `charge` is 6-arg.
    #[must_use]
    pub const fn auto_capture(&self) -> bool {
        matches!(self.auto_capture, Some(true))
    }
}

/// EIP-3009 `ReceiveWithAuthorization` body inside the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthCaptureEip3009Authorization {
    /// Payer.
    pub from: Address,
    /// Must be [`EIP3009_TOKEN_COLLECTOR_ADDRESS`].
    pub to: Address,
    /// Amount (== requirements.amount).
    pub value: TokenAmount,
    /// Always `0` for this scheme.
    pub valid_after: UnixTimestamp,
    /// `preApprovalExpiry` (= now + maxTimeoutSeconds at sign time).
    pub valid_before: UnixTimestamp,
    /// Payer-agnostic `PaymentInfo` hash.
    pub nonce: B256,
}

/// EIP-3009-shaped auth-capture payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthCaptureEip3009Payload {
    /// Authorization that was signed.
    pub authorization: AuthCaptureEip3009Authorization,
    /// 65-byte ECDSA signature (r, s, v).
    pub signature: Bytes,
    /// Fresh client salt (bytes32).
    pub salt: B256,
}

/// Permit2 token permissions for auth-capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthCaptureTokenPermissions {
    /// Token contract.
    pub token: Address,
    /// Amount.
    pub amount: TokenAmount,
}

/// Permit2 authorization (no witness — nonce binds `PaymentInfo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthCapturePermit2Authorization {
    /// Payer.
    pub from: Address,
    /// Token + amount.
    pub permitted: AuthCaptureTokenPermissions,
    /// Must be [`PERMIT2_TOKEN_COLLECTOR_ADDRESS`].
    pub spender: Address,
    /// `uint256` form of payer-agnostic `PaymentInfo` hash.
    pub nonce: TokenAmount,
    /// `preApprovalExpiry`.
    pub deadline: TokenAmount,
}

/// Permit2-shaped auth-capture payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthCapturePermit2Payload {
    /// Permit2 fields that were signed.
    pub permit2_authorization: AuthCapturePermit2Authorization,
    /// 65-byte ECDSA signature (r, s, v).
    pub signature: Bytes,
    /// Fresh client salt.
    pub salt: B256,
}

/// Unified auth-capture payload (Permit2 tried first — unique field name).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AuthCapturePayload {
    /// Permit2 collector path.
    Permit2(AuthCapturePermit2Payload),
    /// EIP-3009 collector path.
    Eip3009(AuthCaptureEip3009Payload),
}

impl AuthCapturePayload {
    /// Payer address.
    #[must_use]
    pub const fn payer(&self) -> Address {
        match self {
            Self::Eip3009(p) => p.authorization.from,
            Self::Permit2(p) => p.permit2_authorization.from,
        }
    }

    /// Client salt.
    #[must_use]
    pub const fn salt(&self) -> B256 {
        match self {
            Self::Eip3009(p) => p.salt,
            Self::Permit2(p) => p.salt,
        }
    }

    /// Signature bytes.
    #[must_use]
    pub const fn signature(&self) -> &Bytes {
        match self {
            Self::Eip3009(p) => &p.signature,
            Self::Permit2(p) => &p.signature,
        }
    }

    /// Transfer method implied by payload shape.
    #[must_use]
    pub const fn transfer_method(&self) -> AssetTransferMethod {
        match self {
            Self::Eip3009(_) => AssetTransferMethod::Eip3009,
            Self::Permit2(_) => AssetTransferMethod::Permit2,
        }
    }
}

/// On-chain `PaymentInfo` (Solidity field names — typehash must match escrow).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentInfo {
    /// `extra.captureAuthorizer`.
    pub operator: Address,
    /// Payer (zeroed for nonce hash).
    pub payer: Address,
    /// `requirements.payTo`.
    pub receiver: Address,
    /// `requirements.asset`.
    pub token: Address,
    /// `requirements.amount` as uint120 (stored as U256).
    pub max_amount: alloy_primitives::U256,
    /// `now + maxTimeoutSeconds` at sign time.
    pub pre_approval_expiry: u64,
    /// `extra.captureDeadline`.
    pub authorization_expiry: u64,
    /// `extra.refundDeadline`.
    pub refund_expiry: u64,
    /// `extra.minFeeBps`.
    pub min_fee_bps: u16,
    /// `extra.maxFeeBps`.
    pub max_fee_bps: u16,
    /// `extra.feeRecipient`.
    pub fee_receiver: Address,
    /// Client salt as uint256.
    pub salt: alloy_primitives::U256,
}

/// EIP-712 structs shared by client signing and off-chain verify.
#[cfg(any(feature = "client", feature = "facilitator"))]
use alloy_sol_types::sol;

#[cfg(any(feature = "client", feature = "facilitator"))]
sol! {
    struct ReceiveWithAuthorization {
        address from;
        address to;
        uint256 value;
        uint256 validAfter;
        uint256 validBefore;
        bytes32 nonce;
    }

    struct TokenPermissions {
        address token;
        uint256 amount;
    }

    struct PermitTransferFrom {
        TokenPermissions permitted;
        address spender;
        uint256 nonce;
        uint256 deadline;
    }
}

/// Wire aliases for typed verify/settle.
pub mod v2 {
    use r402_protocol::payment as wire;

    use super::{AuthCaptureExtra, AuthCapturePayload, AuthCaptureScheme};
    use crate::chain::{ChecksummedAddress, TokenAmount};

    /// Typed verify request.
    pub type VerifyRequest = wire::TypedVerifyRequest<2, PaymentPayload, PaymentRequirements>;
    /// Typed settle request.
    pub type SettleRequest = VerifyRequest;
    /// Payment payload.
    pub type PaymentPayload = wire::PaymentPayload<PaymentRequirements, AuthCapturePayload>;
    /// Payment requirements.
    pub type PaymentRequirements = wire::PaymentRequirements<
        AuthCaptureScheme,
        TokenAmount,
        ChecksummedAddress,
        AuthCaptureExtra,
    >;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_addresses_checksum() {
        assert_eq!(
            AUTH_CAPTURE_ESCROW_ADDRESS.to_checksum(None),
            "0xBdEA0D1bcC5966192B070Fdf62aB4EF5b4420cff"
        );
        assert_eq!(
            EIP3009_TOKEN_COLLECTOR_ADDRESS.to_checksum(None),
            "0x0E3dF9510de65469C4518D7843919c0b8C7A7757"
        );
        assert_eq!(
            PERMIT2_TOKEN_COLLECTOR_ADDRESS.to_checksum(None),
            "0x992476B9Ee81d52a5BdA0622C333938D0Af0aB26"
        );
    }
}
