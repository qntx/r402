//! Wire types and constants for EVM `auth-capture`.
//!
//! Addresses and field names match
//! `specs/schemes/auth-capture/scheme_auth_capture_evm.md` and official
//! `auth-capture/constants.ts`. Default aliases are commerce-payments **v1.1**.

use alloy_primitives::{Address, B256, Bytes, U256, address};
use r402_protocol::error::VerificationError;
use r402_protocol::payment::UnixTimestamp;
pub use r402_protocol::scheme::AuthCaptureScheme;
use serde::{Deserialize, Serialize};

use crate::asset::AssetTransferMethod;
use crate::chain::{ChecksummedAddress, TokenAmount};

/// Official extra-reject reason when `authCaptureEscrow` is not a known escrow.
pub const INVALID_AUTH_CAPTURE_EVM_EXTRA: &str = "invalid_auth_capture_evm_extra";
/// Official payload-reject reason for mixed or deployment-wrong fee fields.
pub const INVALID_AUTH_CAPTURE_EVM_PAYLOAD_FORMAT: &str = "invalid_auth_capture_evm_payload_format";

// commerce-payments v1.0 (tag v1.0.0)
/// v1.0 `AuthCaptureEscrow`.
pub const AUTH_CAPTURE_ESCROW_V1_0_ADDRESS: Address =
    address!("0xBdEA0D1bcC5966192B070Fdf62aB4EF5b4420cff");
/// v1.0 EIP-3009 token collector.
pub const EIP3009_TOKEN_COLLECTOR_V1_0_ADDRESS: Address =
    address!("0x0E3dF9510de65469C4518D7843919c0b8C7A7757");
/// v1.0 Permit2 token collector.
pub const PERMIT2_TOKEN_COLLECTOR_V1_0_ADDRESS: Address =
    address!("0x992476B9Ee81d52a5BdA0622C333938D0Af0aB26");
/// v1.0 operator refund collector.
pub const OPERATOR_REFUND_COLLECTOR_V1_0_ADDRESS: Address =
    address!("0x934907bffd0901b6A21e398B9C53A4A38F02fa5d");

// commerce-payments v1.1 (default)
/// v1.1 `AuthCaptureEscrow`.
pub const AUTH_CAPTURE_ESCROW_V1_1_ADDRESS: Address =
    address!("0x13AC3b34322D12FE27D5e192D0c2b2266d4F29CB");
/// v1.1 EIP-3009 token collector.
pub const EIP3009_TOKEN_COLLECTOR_V1_1_ADDRESS: Address =
    address!("0xEA902B37036bcb4944577ec2101ABdEDF56EbD28");
/// v1.1 Permit2 token collector.
pub const PERMIT2_TOKEN_COLLECTOR_V1_1_ADDRESS: Address =
    address!("0x1aacb38b16a1a8709e80746825E53A0C9Cae9b70");
/// v1.1 operator refund collector.
pub const OPERATOR_REFUND_COLLECTOR_V1_1_ADDRESS: Address =
    address!("0x6a1ADdEEb4bD9c5811a613e20c172b6CE61A4aaB");

/// Default deployment aliases (v1.1).
pub const AUTH_CAPTURE_ESCROW_ADDRESS: Address = AUTH_CAPTURE_ESCROW_V1_1_ADDRESS;
/// Default EIP-3009 collector (v1.1).
pub const EIP3009_TOKEN_COLLECTOR_ADDRESS: Address = EIP3009_TOKEN_COLLECTOR_V1_1_ADDRESS;
/// Default Permit2 collector (v1.1).
pub const PERMIT2_TOKEN_COLLECTOR_ADDRESS: Address = PERMIT2_TOKEN_COLLECTOR_V1_1_ADDRESS;
/// Default operator refund collector (v1.1).
pub const OPERATOR_REFUND_COLLECTOR_ADDRESS: Address = OPERATOR_REFUND_COLLECTOR_V1_1_ADDRESS;

/// Clock-skew buffer (seconds) for deadline checks — matches foundation SDKs.
pub const AUTH_CAPTURE_CLOCK_SKEW_SECS: u64 = 6;

/// Shared operator EIP-712 domain `name`.
pub const OPERATOR_EIP712_NAME: &str = "x402 Auth Capture Operator";
/// Shared operator EIP-712 domain `version`.
pub const OPERATOR_EIP712_VERSION: &str = "1";

/// Commerce-payments CREATE2 set version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthCaptureDeploymentVersion {
    /// commerce-payments v1.0 — charge/capture submit `feeBps`.
    V1_0,
    /// commerce-payments v1.1 — charge/capture submit `feeAmount`.
    V1_1,
}

/// Canonical escrow + collectors for one commerce-payments CREATE2 set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthCaptureDeployment {
    /// Deployment version (selects fee encoding).
    pub version: AuthCaptureDeploymentVersion,
    /// `AuthCaptureEscrow` bound into the signature nonce.
    pub escrow: Address,
    /// EIP-3009 `authorization.to`. Never read from extra.
    pub eip3009_collector: Address,
    /// Permit2 `spender`. Never read from extra.
    pub permit2_collector: Address,
    /// Operator refund collector.
    pub operator_refund_collector: Address,
}

/// commerce-payments v1.0 deployment.
pub const AUTH_CAPTURE_DEPLOYMENT_V1_0: AuthCaptureDeployment = AuthCaptureDeployment {
    version: AuthCaptureDeploymentVersion::V1_0,
    escrow: AUTH_CAPTURE_ESCROW_V1_0_ADDRESS,
    eip3009_collector: EIP3009_TOKEN_COLLECTOR_V1_0_ADDRESS,
    permit2_collector: PERMIT2_TOKEN_COLLECTOR_V1_0_ADDRESS,
    operator_refund_collector: OPERATOR_REFUND_COLLECTOR_V1_0_ADDRESS,
};

/// commerce-payments v1.1 deployment (default).
pub const AUTH_CAPTURE_DEPLOYMENT_V1_1: AuthCaptureDeployment = AuthCaptureDeployment {
    version: AuthCaptureDeploymentVersion::V1_1,
    escrow: AUTH_CAPTURE_ESCROW_V1_1_ADDRESS,
    eip3009_collector: EIP3009_TOKEN_COLLECTOR_V1_1_ADDRESS,
    permit2_collector: PERMIT2_TOKEN_COLLECTOR_V1_1_ADDRESS,
    operator_refund_collector: OPERATOR_REFUND_COLLECTOR_V1_1_ADDRESS,
};

impl AuthCaptureDeployment {
    /// Collector for the payload's transfer method.
    #[must_use]
    pub const fn collector(self, method: AssetTransferMethod) -> Address {
        match method {
            AssetTransferMethod::Eip3009 => self.eip3009_collector,
            AssetTransferMethod::Permit2 => self.permit2_collector,
        }
    }
}

/// Resolve the commerce-payments deployment from optional `extra.authCaptureEscrow`.
///
/// Absent → v1.1. The v1.1 escrow → v1.1. The v1.0 escrow → v1.0.
/// Any other address → `None` (caller rejects; no v1.0-as-default fallback).
#[must_use]
pub fn resolve_auth_capture_deployment(escrow: Option<Address>) -> Option<AuthCaptureDeployment> {
    match escrow {
        None => Some(AUTH_CAPTURE_DEPLOYMENT_V1_1),
        Some(addr)
            if addr == AUTH_CAPTURE_ESCROW_V1_1_ADDRESS || addr == AUTH_CAPTURE_ESCROW_ADDRESS =>
        {
            Some(AUTH_CAPTURE_DEPLOYMENT_V1_1)
        }
        Some(addr) if addr == AUTH_CAPTURE_ESCROW_V1_0_ADDRESS => {
            Some(AUTH_CAPTURE_DEPLOYMENT_V1_0)
        }
        Some(_) => None,
    }
}

/// Integer fee from amount and bps (same division the escrow uses).
#[must_use]
pub fn fee_amount_from_bps(amount: U256, fee_bps: u16) -> U256 {
    (amount * U256::from(fee_bps)) / U256::from(10_000u16)
}

/// Submitted `charge` / `capture` fee for a resolved deployment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    variant_size_differences,
    reason = "official wire is feeBps (u16) XOR feeAmount (U256)"
)]
pub enum SubmittedFee {
    /// v1.0 pin: `feeBps` (`uint16`).
    Bps(u16),
    /// v1.1: absolute `feeAmount` in atomic units.
    Amount(TokenAmount),
}

/// Default submitted fee: v1.1 `feeAmount = amount * minFeeBps / 10000`; v1.0 pins `feeBps`.
#[must_use]
pub fn default_submitted_fee(
    version: AuthCaptureDeploymentVersion,
    amount: U256,
    min_fee_bps: u16,
) -> SubmittedFee {
    match version {
        AuthCaptureDeploymentVersion::V1_1 => {
            SubmittedFee::Amount(TokenAmount(fee_amount_from_bps(amount, min_fee_bps)))
        }
        AuthCaptureDeploymentVersion::V1_0 => SubmittedFee::Bps(min_fee_bps),
    }
}

/// Decode wire `feeBps` / `feeAmount` against the resolved deployment.
///
/// Neither field → authorize (no submitted fee). Both, or the field that does
/// not belong to the deployment → `invalid_auth_capture_evm_payload_format`.
///
/// # Errors
///
/// Mixed fee fields, `feeBps` on v1.1, or `feeAmount` on a v1.0 pin.
pub fn submitted_fee_from_wire(
    version: AuthCaptureDeploymentVersion,
    fee_bps: Option<u16>,
    fee_amount: Option<TokenAmount>,
) -> Result<Option<SubmittedFee>, VerificationError> {
    match (version, fee_bps, fee_amount) {
        (_, None, None) => Ok(None),
        (AuthCaptureDeploymentVersion::V1_1, None, Some(amount)) => {
            Ok(Some(SubmittedFee::Amount(amount)))
        }
        (AuthCaptureDeploymentVersion::V1_0, Some(bps), None) => Ok(Some(SubmittedFee::Bps(bps))),
        (AuthCaptureDeploymentVersion::V1_1, Some(_), None)
        | (AuthCaptureDeploymentVersion::V1_0, None, Some(_))
        | (_, Some(_), Some(_)) => Err(VerificationError::from_wire(
            INVALID_AUTH_CAPTURE_EVM_PAYLOAD_FORMAT,
        )),
    }
}

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
    /// `true` is rejected at verify (`autoCapture` is unsupported).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_capture: Option<bool>,
    /// Transfer method; default eip3009 when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_transfer_method: Option<AssetTransferMethod>,
    /// Selects the commerce-payments deployment. Absent → v1.1. Collectors are
    /// never carried here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_capture_escrow: Option<ChecksummedAddress>,
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

    /// Whether `extra.autoCapture` is `true` (rejected at verify).
    #[must_use]
    pub const fn auto_capture(&self) -> bool {
        matches!(self.auto_capture, Some(true))
    }

    /// Resolved deployment from `authCaptureEscrow`. `None` when the address is unknown.
    #[must_use]
    pub fn deployment(&self) -> Option<AuthCaptureDeployment> {
        resolve_auth_capture_deployment(self.auth_capture_escrow.map(|a| a.0))
    }

    /// Resolved deployment, or the official extra-reject reason.
    ///
    /// # Errors
    ///
    /// Unknown `authCaptureEscrow`.
    pub fn require_deployment(&self) -> Result<AuthCaptureDeployment, VerificationError> {
        self.deployment()
            .ok_or_else(|| VerificationError::from_wire(INVALID_AUTH_CAPTURE_EVM_EXTRA))
    }
}

/// EIP-3009 `ReceiveWithAuthorization` body inside the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthCaptureEip3009Authorization {
    /// Payer.
    pub from: Address,
    /// Must be the resolved deployment's EIP-3009 collector.
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
    /// v1.0 charge/capture `feeBps`. Mutually exclusive with [`Self::fee_amount`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_bps: Option<u16>,
    /// v1.1 charge/capture `feeAmount`. Mutually exclusive with [`Self::fee_bps`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_amount: Option<TokenAmount>,
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
    /// Must be the resolved deployment's Permit2 collector.
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
    /// v1.0 charge/capture `feeBps`. Mutually exclusive with [`Self::fee_amount`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_bps: Option<u16>,
    /// v1.1 charge/capture `feeAmount`. Mutually exclusive with [`Self::fee_bps`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_amount: Option<TokenAmount>,
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

    /// Wire `feeBps`, if present.
    #[must_use]
    pub const fn fee_bps(&self) -> Option<u16> {
        match self {
            Self::Eip3009(p) => p.fee_bps,
            Self::Permit2(p) => p.fee_bps,
        }
    }

    /// Wire `feeAmount`, if present.
    #[must_use]
    pub const fn fee_amount(&self) -> Option<TokenAmount> {
        match self {
            Self::Eip3009(p) => p.fee_amount,
            Self::Permit2(p) => p.fee_amount,
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
    pub max_amount: U256,
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
    pub salt: U256,
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

    /// v1.0 operator Charge (feeBps).
    struct ChargeV1_0 {
        bytes32 paymentInfoHash;
        uint256 amount;
        address tokenCollector;
        bytes32 collectorDataHash;
        uint16 feeBps;
        address feeReceiver;
    }

    /// v1.1 operator Charge (feeAmount). Default Charge type.
    struct ChargeV1_1 {
        bytes32 paymentInfoHash;
        uint256 amount;
        address tokenCollector;
        bytes32 collectorDataHash;
        uint256 feeAmount;
        address feeReceiver;
    }

    /// v1.0 operator Capture (feeBps).
    struct CaptureV1_0 {
        bytes32 paymentInfoHash;
        uint256 amount;
        uint16 feeBps;
        address feeReceiver;
        uint256 expectedCapturableAmount;
        uint256 expectedRefundableAmount;
    }

    /// v1.1 operator Capture (feeAmount). Default Capture type.
    struct CaptureV1_1 {
        bytes32 paymentInfoHash;
        uint256 amount;
        uint256 feeAmount;
        address feeReceiver;
        uint256 expectedCapturableAmount;
        uint256 expectedRefundableAmount;
    }
}

/// Default v1.1 Charge EIP-712 type.
#[cfg(any(feature = "client", feature = "facilitator"))]
pub type Charge = ChargeV1_1;
/// Default v1.1 Capture EIP-712 type.
#[cfg(any(feature = "client", feature = "facilitator"))]
pub type Capture = CaptureV1_1;

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
            "0x13AC3b34322D12FE27D5e192D0c2b2266d4F29CB"
        );
        assert_eq!(
            EIP3009_TOKEN_COLLECTOR_ADDRESS.to_checksum(None),
            "0xEA902B37036bcb4944577ec2101ABdEDF56EbD28"
        );
        assert_eq!(
            PERMIT2_TOKEN_COLLECTOR_ADDRESS.to_checksum(None),
            "0x1aacb38b16a1a8709e80746825E53A0C9Cae9b70"
        );
        assert_eq!(
            OPERATOR_REFUND_COLLECTOR_ADDRESS.to_checksum(None),
            "0x6a1ADdEEb4bD9c5811a613e20c172b6CE61A4aaB"
        );
    }

    #[test]
    fn v1_0_named_constants_checksum() {
        assert_eq!(
            AUTH_CAPTURE_ESCROW_V1_0_ADDRESS.to_checksum(None),
            "0xBdEA0D1bcC5966192B070Fdf62aB4EF5b4420cff"
        );
        assert_eq!(
            EIP3009_TOKEN_COLLECTOR_V1_0_ADDRESS.to_checksum(None),
            "0x0E3dF9510de65469C4518D7843919c0b8C7A7757"
        );
        assert_eq!(
            PERMIT2_TOKEN_COLLECTOR_V1_0_ADDRESS.to_checksum(None),
            "0x992476B9Ee81d52a5BdA0622C333938D0Af0aB26"
        );
        assert_eq!(
            OPERATOR_REFUND_COLLECTOR_V1_0_ADDRESS.to_checksum(None),
            "0x934907bffd0901b6A21e398B9C53A4A38F02fa5d"
        );
    }

    #[test]
    fn aliases_are_v1_1_not_v1_0() {
        assert_eq!(
            AUTH_CAPTURE_ESCROW_ADDRESS,
            AUTH_CAPTURE_ESCROW_V1_1_ADDRESS
        );
        assert_ne!(
            AUTH_CAPTURE_ESCROW_ADDRESS,
            AUTH_CAPTURE_ESCROW_V1_0_ADDRESS
        );
        assert_eq!(
            EIP3009_TOKEN_COLLECTOR_ADDRESS,
            EIP3009_TOKEN_COLLECTOR_V1_1_ADDRESS
        );
        assert_eq!(
            PERMIT2_TOKEN_COLLECTOR_ADDRESS,
            PERMIT2_TOKEN_COLLECTOR_V1_1_ADDRESS
        );
    }

    #[test]
    fn resolve_absent_is_v1_1() {
        let d = resolve_auth_capture_deployment(None).expect("default");
        assert_eq!(d, AUTH_CAPTURE_DEPLOYMENT_V1_1);
        assert_eq!(d.version, AuthCaptureDeploymentVersion::V1_1);
    }

    #[test]
    fn resolve_v1_1_escrow_is_v1_1() {
        let d = resolve_auth_capture_deployment(Some(AUTH_CAPTURE_ESCROW_V1_1_ADDRESS))
            .expect("v1.1 escrow");
        assert_eq!(d, AUTH_CAPTURE_DEPLOYMENT_V1_1);
    }

    #[test]
    fn resolve_v1_0_escrow_is_v1_0() {
        let d = resolve_auth_capture_deployment(Some(AUTH_CAPTURE_ESCROW_V1_0_ADDRESS))
            .expect("v1.0 escrow");
        assert_eq!(d, AUTH_CAPTURE_DEPLOYMENT_V1_0);
        assert_eq!(d.eip3009_collector, EIP3009_TOKEN_COLLECTOR_V1_0_ADDRESS);
        assert_eq!(d.permit2_collector, PERMIT2_TOKEN_COLLECTOR_V1_0_ADDRESS);
    }

    #[test]
    fn resolve_unknown_escrow_is_none() {
        assert_eq!(
            resolve_auth_capture_deployment(Some(Address::repeat_byte(0x99))),
            None,
            "unknown escrow must not fall back to v1.0 or v1.1"
        );
    }

    #[test]
    fn fee_amount_from_bps_matches_escrow_integer_div() {
        assert_eq!(
            fee_amount_from_bps(U256::from(1_000_000_u64), 100),
            U256::from(10_000_u64)
        );
        assert_eq!(
            fee_amount_from_bps(U256::from(1_u64), 1),
            U256::ZERO,
            "escrow integer division truncates"
        );
        assert_eq!(
            default_submitted_fee(
                AuthCaptureDeploymentVersion::V1_1,
                U256::from(1_000_000_u64),
                100
            ),
            SubmittedFee::Amount(TokenAmount(U256::from(10_000_u64)))
        );
        assert_eq!(
            default_submitted_fee(
                AuthCaptureDeploymentVersion::V1_0,
                U256::from(1_000_000_u64),
                100
            ),
            SubmittedFee::Bps(100)
        );
    }

    #[test]
    fn submitted_fee_rejects_mixed_and_wrong_deployment_field() {
        let amount = TokenAmount(U256::from(10_u64));
        assert!(
            submitted_fee_from_wire(AuthCaptureDeploymentVersion::V1_1, Some(1), Some(amount))
                .is_err()
        );
        assert!(
            submitted_fee_from_wire(AuthCaptureDeploymentVersion::V1_1, Some(1), None).is_err(),
            "v1.1 rejects feeBps"
        );
        assert!(
            submitted_fee_from_wire(AuthCaptureDeploymentVersion::V1_0, None, Some(amount))
                .is_err(),
            "v1.0 pin rejects feeAmount"
        );
        assert_eq!(
            submitted_fee_from_wire(AuthCaptureDeploymentVersion::V1_1, None, Some(amount))
                .expect("v1.1 feeAmount"),
            Some(SubmittedFee::Amount(amount))
        );
        assert_eq!(
            submitted_fee_from_wire(AuthCaptureDeploymentVersion::V1_0, Some(50), None)
                .expect("v1.0 feeBps"),
            Some(SubmittedFee::Bps(50))
        );
        assert_eq!(
            submitted_fee_from_wire(AuthCaptureDeploymentVersion::V1_1, None, None)
                .expect("authorize"),
            None
        );
    }

    #[test]
    fn collectors_are_not_read_from_extra() {
        let extra: AuthCaptureExtra = serde_json::from_value(serde_json::json!({
            "name": "USD Coin",
            "version": "2",
            "captureAuthorizer": "0x1111111111111111111111111111111111111111",
            "captureDeadline": 1,
            "refundDeadline": 2,
            "feeRecipient": "0x2222222222222222222222222222222222222222",
            "minFeeBps": 0,
            "maxFeeBps": 0,
            "eip3009Collector": "0x0E3dF9510de65469C4518D7843919c0b8C7A7757",
            "permit2Collector": "0x992476B9Ee81d52a5BdA0622C333938D0Af0aB26"
        }))
        .expect("unknown collector keys ignored");
        let d = extra.deployment().expect("omitted escrow is v1.1");
        assert_eq!(d.eip3009_collector, EIP3009_TOKEN_COLLECTOR_V1_1_ADDRESS);
        assert_eq!(d.permit2_collector, PERMIT2_TOKEN_COLLECTOR_V1_1_ADDRESS);
        assert_eq!(d.escrow, AUTH_CAPTURE_ESCROW_V1_1_ADDRESS);
    }
}
