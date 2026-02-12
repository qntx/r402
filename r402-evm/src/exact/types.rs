//! Type definitions for the EIP-155 "exact" payment scheme.
//!
//! This module defines shared wire format types for EVM exact payments,
//! supporting both EIP-3009 (`transferWithAuthorization`) and Permit2
//! transfer methods. Wire format type aliases live in the [`v2`] sub-module.

use alloy_primitives::{Address, B256, Bytes, address};
#[cfg(any(feature = "facilitator", feature = "client"))]
use alloy_sol_types::sol;
use r402::proto::UnixTimestamp;
pub use r402::scheme::ExactScheme;
use serde::{Deserialize, Serialize};

use crate::chain::TokenAmount;

/// Canonical Uniswap Permit2 contract address (same on all EVM chains via CREATE2).
pub const PERMIT2_ADDRESS: Address = address!("0x000000000022D473030F116dDEE9F6B43aC78BA3");

/// x402 exact payment Permit2 proxy contract address.
pub const X402_EXACT_PERMIT2_PROXY: Address =
    address!("0x4020615294c913F045dc10f0a5cdEbd86c280001");

/// x402 upto payment Permit2 proxy contract address.
pub const X402_UPTO_PERMIT2_PROXY: Address = address!("0x4020633461b2895a48930Ff97eE8fCdE8E520002");

/// Determines which on-chain mechanism is used for token transfers.
///
/// - `Eip3009`: Uses `transferWithAuthorization` (USDC, etc.) — recommended for compatible tokens
/// - `Permit2`: Uses Permit2 + `x402Permit2Proxy` — universal fallback for any ERC-20
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AssetTransferMethod {
    /// EIP-3009 `transferWithAuthorization`.
    Eip3009,
    /// Uniswap Permit2 via `x402Permit2Proxy`.
    Permit2,
}

/// Unified exact payment payload — either EIP-3009 or Permit2.
///
/// Deserialization uses `#[serde(untagged)]`: the Permit2 variant is tried first
/// because it contains the unique `permit2Authorization` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExactPayload {
    /// Permit2-based payment (tried first during deserialization).
    Permit2(Permit2Payload),
    /// EIP-3009-based payment.
    Eip3009(Eip3009Payload),
}

impl ExactPayload {
    /// Returns the transfer method used by this payload.
    #[must_use]
    pub const fn transfer_method(&self) -> AssetTransferMethod {
        match self {
            Self::Eip3009(_) => AssetTransferMethod::Eip3009,
            Self::Permit2(_) => AssetTransferMethod::Permit2,
        }
    }

    /// Returns the sender (`from`) address for this payment.
    #[must_use]
    pub const fn from_address(&self) -> Address {
        match self {
            Self::Eip3009(p) => p.authorization.from,
            Self::Permit2(p) => p.permit2_authorization.from,
        }
    }

    /// Returns the raw signature bytes.
    pub const fn signature(&self) -> &Bytes {
        match self {
            Self::Eip3009(p) => &p.signature,
            Self::Permit2(p) => &p.signature,
        }
    }
}

/// EIP-3009 `transferWithAuthorization` payment payload.
///
/// Contains both the EIP-712 signature and the structured authorization
/// data. Together, they provide everything needed to execute a
/// `transferWithAuthorization` call on an ERC-3009 compliant token contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Eip3009Payload {
    /// The cryptographic signature authorizing the transfer.
    ///
    /// This can be:
    /// - An EOA signature (64-65 bytes, split into r, s, v components)
    /// - An EIP-1271 signature (arbitrary length, validated by contract)
    /// - An EIP-6492 signature (wrapped with deployment data and magic suffix)
    pub signature: Bytes,

    /// The structured authorization data that was signed.
    pub authorization: Eip3009Authorization,
}

/// Permit2 token permissions — which token and how much.
///
/// Part of the `PermitWitnessTransferFrom` message structure that gets signed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Permit2TokenPermissions {
    /// Token contract address.
    pub token: Address,
    /// Amount in smallest unit as decimal string (e.g., `"1000000"` for 1 USDC).
    pub amount: TokenAmount,
}

/// Witness data verified on-chain by `x402Permit2Proxy`.
///
/// Included in the EIP-712 signature and checked by the proxy contract.
/// Note: upper time bound is enforced by Permit2's `deadline` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Permit2Witness {
    /// Destination address for funds.
    pub to: Address,
    /// Unix timestamp — payment invalid before this time.
    pub valid_after: TokenAmount,
    /// Extra data (typically `0x` for empty).
    pub extra: Bytes,
}

/// Permit2 authorization parameters.
///
/// Maps to the `PermitWitnessTransferFrom` struct used by the Permit2 contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Permit2Authorization {
    /// Signer / owner address.
    pub from: Address,
    /// Token and amount permitted.
    pub permitted: Permit2TokenPermissions,
    /// Must be the `x402Permit2Proxy` address.
    pub spender: Address,
    /// Unique nonce (uint256 as decimal string).
    pub nonce: TokenAmount,
    /// Signature expires after this unix timestamp (uint256 as decimal string).
    pub deadline: TokenAmount,
    /// Witness data verified by `x402Permit2Proxy`.
    pub witness: Permit2Witness,
}

/// Permit2 payment payload sent by clients.
///
/// Contains the EIP-712 signature over a `PermitWitnessTransferFrom`
/// and the authorization parameters that were signed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Permit2Payload {
    /// EIP-712 signature (hex, 65 bytes for EOA).
    pub signature: Bytes,
    /// Authorization parameters that were signed.
    pub permit2_authorization: Permit2Authorization,
}

/// EIP-712 structured data for ERC-3009 transfer authorization.
///
/// This struct defines the parameters of a `transferWithAuthorization` call:
/// who can transfer tokens, to whom, how much, and during what time window.
/// The struct is signed using EIP-712 typed data signing.
#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Eip3009Authorization {
    /// The address authorizing the transfer (token owner).
    pub from: Address,

    /// The recipient address for the transfer.
    pub to: Address,

    /// The amount of tokens to transfer (in token's smallest unit).
    pub value: TokenAmount,

    /// The authorization is not valid before this timestamp (inclusive).
    pub valid_after: UnixTimestamp,

    /// The authorization expires at this timestamp (exclusive).
    pub valid_before: UnixTimestamp,

    /// A unique 32-byte nonce to prevent replay attacks.
    pub nonce: B256,
}

/// Extra payment requirements data for the EVM exact scheme.
///
/// Contains optional EIP-712 domain parameters and the asset transfer method.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequirementsExtra {
    /// The token name as used in the EIP-712 domain (required for EIP-3009).
    pub name: String,

    /// The token version as used in the EIP-712 domain (required for EIP-3009).
    pub version: String,

    /// Which on-chain transfer mechanism to use.
    ///
    /// - `Some(Eip3009)` or `None` → EIP-3009 `transferWithAuthorization`
    /// - `Some(Permit2)` → Permit2 via `x402Permit2Proxy`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_transfer_method: Option<AssetTransferMethod>,
}

impl PaymentRequirementsExtra {
    /// Builds the serialized `extra` JSON from EIP-712 deployment data and
    /// an optional transfer method.
    ///
    /// Returns `None` when both `eip712` and `method` are absent (pure default).
    #[must_use]
    pub fn from_deployment(
        eip712: Option<crate::chain::TokenDeploymentEip712>,
        method: Option<AssetTransferMethod>,
    ) -> Option<serde_json::Value> {
        let extra = match (eip712, method) {
            (Some(eip712), method) => Self::from(eip712).with_transfer_method(method),
            (None, Some(m)) => Self {
                name: String::new(),
                version: String::new(),
                asset_transfer_method: Some(m),
            },
            (None, None) => return None,
        };
        serde_json::to_value(extra).ok()
    }

    /// Sets the asset transfer method, consuming and returning `self`.
    #[must_use]
    pub const fn with_transfer_method(mut self, method: Option<AssetTransferMethod>) -> Self {
        self.asset_transfer_method = method;
        self
    }
}

impl From<crate::chain::TokenDeploymentEip712> for PaymentRequirementsExtra {
    fn from(eip712: crate::chain::TokenDeploymentEip712) -> Self {
        Self {
            name: eip712.name,
            version: eip712.version,
            asset_transfer_method: None,
        }
    }
}

#[cfg(any(feature = "facilitator", feature = "client"))]
sol!(
    /// Solidity-compatible struct definition for ERC-3009 `transferWithAuthorization`.
    ///
    /// This matches the EIP-3009 format used in EIP-712 typed data:
    /// it defines the authorization to transfer tokens from `from` to `to`
    /// for a specific `value`, valid only between `validAfter` and `validBefore`
    /// and identified by a unique `nonce`.
    ///
    /// This struct is primarily used to reconstruct the typed data domain/message
    /// when verifying a client's signature.
    #[derive(Serialize, Deserialize)]
    struct TransferWithAuthorization {
        address from;
        address to;
        uint256 value;
        uint256 validAfter;
        uint256 validBefore;
        bytes32 nonce;
    }
);

#[cfg(any(feature = "facilitator", feature = "client"))]
sol!(
    /// EIP-712 struct for Permit2 token permissions.
    #[derive(Serialize, Deserialize)]
    struct TokenPermissions {
        address token;
        uint256 amount;
    }

    /// EIP-712 struct for x402 Permit2 witness data.
    ///
    /// Field order MUST match the on-chain Permit2 contract definition.
    #[derive(Serialize, Deserialize)]
    struct Witness {
        address to;
        uint256 validAfter;
        bytes extra;
    }

    /// EIP-712 struct for Permit2 `PermitWitnessTransferFrom`.
    ///
    /// This is the primary type signed by the payer when using Permit2.
    /// The domain uses `name = "Permit2"`, no version, and
    /// `verifyingContract = PERMIT2_ADDRESS`.
    #[derive(Serialize, Deserialize)]
    struct PermitWitnessTransferFrom {
        TokenPermissions permitted;
        address spender;
        uint256 nonce;
        uint256 deadline;
        Witness witness;
    }
);

/// Wire format type aliases for EIP-155 exact scheme.
///
/// Uses CAIP-2 chain IDs (e.g., `eip155:8453`) for chain identification
/// and embeds requirements directly in the payload.
pub mod v2 {
    use r402::proto::v2 as proto_v2;

    use super::{ExactPayload, ExactScheme, PaymentRequirementsExtra};
    use crate::chain::{ChecksummedAddress, TokenAmount};

    /// Type alias for verify requests using the exact EVM payment scheme.
    pub type VerifyRequest = proto_v2::VerifyRequest<PaymentPayload, PaymentRequirements>;

    /// Type alias for settle requests (same structure as verify requests).
    pub type SettleRequest = VerifyRequest;

    /// Type alias for payment payloads with embedded requirements and EVM-specific data.
    pub type PaymentPayload = proto_v2::PaymentPayload<PaymentRequirements, ExactPayload>;

    /// Type alias for payment requirements with EVM-specific types.
    pub type PaymentRequirements = proto_v2::PaymentRequirements<
        ExactScheme,
        TokenAmount,
        ChecksummedAddress,
        PaymentRequirementsExtra,
    >;
}
