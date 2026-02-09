//! Wire format types for the EVM "exact" payment scheme.
//!
//! Defines the ERC-3009 `transferWithAuthorization` payload structure
//! and the Solidity ABI bindings used for on-chain interactions.
//!
//! Corresponds to Python SDK's EVM exact types and ABIs.

use alloy_sol_types::sol;
use serde::{Deserialize, Serialize};

/// Scheme identifier for the "exact" payment scheme.
pub const SCHEME_EXACT: &str = "exact";

/// Default validity window for ERC-3009 authorizations (10 minutes).
pub const DEFAULT_VALID_AFTER_LEAD_TIME_SECS: u64 = 10 * 60;

/// Default validity period for ERC-3009 authorizations (30 minutes).
pub const DEFAULT_VALID_BEFORE_BUFFER_SECS: u64 = 30 * 60;

/// ERC-3009 authorization payload.
///
/// Contains the parameters for `transferWithAuthorization` as defined in
/// EIP-3009, encoded as an EIP-712 typed data structure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactAuthorization {
    /// Payer address (the `from` in ERC-3009).
    pub from: String,
    /// Recipient address (the `to` in ERC-3009).
    pub to: String,
    /// Transfer amount in smallest unit.
    pub value: String,
    /// Unix timestamp after which the authorization is valid.
    pub valid_after: String,
    /// Unix timestamp before which the authorization is valid.
    pub valid_before: String,
    /// Unique nonce to prevent replay attacks.
    pub nonce: String,
}

/// Complete "exact" scheme payload (authorization + signature).
///
/// This is the `payload` field inside a `PaymentPayload`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactPayload {
    /// ERC-3009 authorization parameters.
    pub authorization: ExactAuthorization,
    /// EIP-712 signature over the authorization (`0x`-prefixed hex).
    pub signature: String,
}

/// Extra fields required in `PaymentRequirements.extra` for the exact scheme.
///
/// EVM-specific EIP-712 domain parameters needed by the client to construct
/// a valid signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactRequirementsExtra {
    /// EIP-712 domain name (e.g., `"USD Coin"`).
    pub name: String,
    /// EIP-712 domain version (e.g., `"2"`).
    pub version: String,
}

sol! {
    /// ERC-3009 `transferWithAuthorization` function.
    #[derive(Debug, PartialEq, Eq)]
    function transferWithAuthorization(
        address from,
        address to,
        uint256 value,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        bytes memory signature
    ) external;

    /// ERC-3009 `authorizationState` view function.
    #[derive(Debug, PartialEq, Eq)]
    function authorizationState(
        address authorizer,
        bytes32 nonce
    ) external view returns (bool);

    /// ERC-20 `balanceOf` view function.
    #[derive(Debug, PartialEq, Eq)]
    function balanceOf(address account) external view returns (uint256);

    /// ERC-1271 `isValidSignature` view function (for smart wallets).
    #[derive(Debug, PartialEq, Eq)]
    function isValidSignature(
        bytes32 hash,
        bytes memory signature
    ) external view returns (bytes4);

    /// EIP-712 `TransferWithAuthorization` typed data.
    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct TransferWithAuthorization {
        address from;
        address to;
        uint256 value;
        uint256 validAfter;
        uint256 validBefore;
        bytes32 nonce;
    }
}
