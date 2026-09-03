//! Type definitions for the Tron "exact" payment scheme.
//!
//! Tron uses TIP-712 for typed-data signing, which is byte-compatible with
//! EIP-712: the same domain separator and struct-hash algorithm apply, and
//! signatures recover via the same secp256k1 `ecrecover` used on Ethereum.
//! This lets the exact scheme reuse EIP-3009 `transferWithAuthorization` and
//! Permit2 `permitWitnessTransferFrom` verbatim, with two differences from
//! `r402-evm`:
//!
//! - Tron has no contract wallets, so only EOA signatures are supported
//!   (no EIP-1271 / EIP-6492).
//! - Addresses are `Base58Check`-encoded ([`crate::chain::Address`]) on the
//!   wire, converted to the raw EVM hex form only at the TIP-712 signing
//!   boundary.

use alloy_primitives::{Address as EvmAddress, B256, Bytes};
#[cfg(any(feature = "facilitator", feature = "client"))]
use alloy_sol_types::sol;
use r402_protocol::payment::UnixTimestamp;
pub use r402_protocol::scheme::ExactScheme;
use serde::{Deserialize, Serialize};

use crate::chain::TronTokenAmount;

/// Which on-chain mechanism is used to move tokens for the `exact` scheme.
///
/// - `Eip3009`: `transferWithAuthorization` — recommended when the token
///   natively supports it (e.g. deployments mirroring USDC's contract).
/// - `Permit2`: SUN.io's Permit2 deployment plus `x402ExactPermit2Proxy` —
///   universal fallback (e.g. Tron's canonical USDT, which lacks EIP-3009).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AssetTransferMethod {
    /// EIP-3009 `transferWithAuthorization`.
    Eip3009,
    /// SUN.io Permit2 via `x402ExactPermit2Proxy`.
    Permit2,
}

/// Unified exact payment payload — either EIP-3009 or Permit2.
///
/// Deserialization uses `#[serde(untagged)]`: the Permit2 variant is tried
/// first because it contains the unique `permit2Authorization` field.
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

    /// Returns the sender (`from`) address for this payment, in raw EVM form
    /// (the form used at the TIP-712 signing layer).
    #[must_use]
    pub const fn sender(&self) -> EvmAddress {
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Eip3009Payload {
    /// The EOA secp256k1 signature authorizing the transfer (65 bytes).
    ///
    /// Tron has no contract wallets, so this is always a plain EOA
    /// signature — unlike `r402-evm`, EIP-1271 / EIP-6492 do not apply.
    pub signature: Bytes,
    /// The structured authorization data that was signed.
    pub authorization: Eip3009Authorization,
}

/// Permit2 token permissions — which token and how much.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Permit2TokenPermissions {
    /// Token contract address (raw EVM hex form).
    pub token: EvmAddress,
    /// Amount in smallest unit as decimal string.
    pub amount: TronTokenAmount,
}

/// Witness data verified on-chain by `x402ExactPermit2Proxy`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Permit2Witness {
    /// Destination address for funds (raw EVM hex form).
    pub to: EvmAddress,
    /// Unix timestamp — payment invalid before this time.
    pub valid_after: TronTokenAmount,
}

/// Permit2 authorization parameters, matching `PermitWitnessTransferFrom`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Permit2Authorization {
    /// Signer / owner address (raw EVM hex form).
    pub from: EvmAddress,
    /// Token and amount permitted.
    pub permitted: Permit2TokenPermissions,
    /// Must be the `x402ExactPermit2Proxy` address.
    pub spender: EvmAddress,
    /// Unique nonce (uint256 as decimal string).
    pub nonce: TronTokenAmount,
    /// Signature expires after this unix timestamp (uint256 as decimal string).
    pub deadline: TronTokenAmount,
    /// Witness data verified by `x402ExactPermit2Proxy`.
    pub witness: Permit2Witness,
}

/// Permit2 payment payload sent by clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Permit2Payload {
    /// TIP-712 signature (65 bytes, EOA-only).
    pub signature: Bytes,
    /// Authorization parameters that were signed.
    pub permit2_authorization: Permit2Authorization,
}

/// TIP-712 structured data for `transferWithAuthorization`.
#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Eip3009Authorization {
    /// The address authorizing the transfer (token owner), raw EVM hex form.
    pub from: EvmAddress,
    /// The recipient address for the transfer, raw EVM hex form.
    pub to: EvmAddress,
    /// The amount of tokens to transfer (in token's smallest unit).
    pub value: TronTokenAmount,
    /// The authorization is not valid before this timestamp (inclusive).
    pub valid_after: UnixTimestamp,
    /// The authorization expires at this timestamp (exclusive).
    pub valid_before: UnixTimestamp,
    /// A unique 32-byte nonce to prevent replay attacks.
    pub nonce: B256,
}

/// Extra payment requirements data for the Tron exact scheme.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequirementsExtra {
    /// The token name as used in the TIP-712 domain (required for EIP-3009).
    pub name: String,
    /// The token version as used in the TIP-712 domain (required for EIP-3009).
    pub version: String,
    /// Which on-chain transfer mechanism to use.
    ///
    /// - `Some(Eip3009)` or `None` → EIP-3009 `transferWithAuthorization`
    /// - `Some(Permit2)` → SUN.io Permit2 via `x402ExactPermit2Proxy`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_transfer_method: Option<AssetTransferMethod>,
}

impl PaymentRequirementsExtra {
    /// Constructs the `extra` block for a given TIP-712 domain and transfer method.
    #[must_use]
    pub const fn new(
        name: String,
        version: String,
        asset_transfer_method: Option<AssetTransferMethod>,
    ) -> Self {
        Self {
            name,
            version,
            asset_transfer_method,
        }
    }

    /// Builds serialized `extra` from TIP-712 domain data and an optional method.
    ///
    /// Returns `None` when both `tip712` and `method` are absent. Permit2-only
    /// tokens (canonical USDT) have no TIP-712 domain; `Some(Permit2)` still
    /// emits `assetTransferMethod` with empty `name`/`version`.
    #[must_use]
    pub fn from_deployment(
        tip712: Option<crate::chain::TokenDeploymentTip712>,
        method: Option<AssetTransferMethod>,
    ) -> Option<serde_json::Value> {
        let extra = match (tip712, method) {
            (Some(tip712), method) => Self::from(tip712).with_transfer_method(method),
            (None, Some(method)) => Self {
                name: String::new(),
                version: String::new(),
                asset_transfer_method: Some(method),
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

impl From<crate::chain::TokenDeploymentTip712> for PaymentRequirementsExtra {
    fn from(tip712: crate::chain::TokenDeploymentTip712) -> Self {
        Self {
            name: tip712.name,
            version: tip712.version,
            asset_transfer_method: None,
        }
    }
}

#[cfg(any(feature = "facilitator", feature = "client"))]
sol!(
    /// Solidity-compatible struct definition for EIP-3009 `transferWithAuthorization`.
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
    /// TIP-712 struct for Permit2 token permissions.
    #[derive(Serialize, Deserialize)]
    struct TronTokenPermissions {
        address token;
        uint256 amount;
    }

    /// TIP-712 struct for x402 Permit2 witness data.
    ///
    /// Field order and typehash MUST match the deployed
    /// `x402ExactPermit2Proxy` contract (`Witness(address to,uint256 validAfter)`).
    #[derive(Serialize, Deserialize)]
    struct TronWitness {
        address to;
        uint256 validAfter;
    }

    /// TIP-712 struct for Permit2 `PermitWitnessTransferFrom`.
    #[derive(Serialize, Deserialize)]
    struct TronPermitWitnessTransferFrom {
        TronTokenPermissions permitted;
        address spender;
        uint256 nonce;
        uint256 deadline;
        TronWitness witness;
    }
);

/// Wire format type aliases for the Tron exact scheme.
///
/// Uses CAIP-2 chain IDs (e.g. `tron:0x2b6653dc`) for chain identification
/// and embeds requirements directly in the payload.
pub mod v2 {
    use r402_protocol::payment as wire;

    use super::{ExactPayload, ExactScheme, PaymentRequirementsExtra};
    use crate::chain::{Address, TronTokenAmount};

    /// Type alias for verify requests using the exact Tron payment scheme.
    pub type VerifyRequest = wire::TypedVerifyRequest<2, PaymentPayload, PaymentRequirements>;

    /// Type alias for settle requests (same structure as verify requests).
    pub type SettleRequest = VerifyRequest;

    /// Type alias for payment payloads with embedded requirements and Tron-specific data.
    pub type PaymentPayload = wire::PaymentPayload<PaymentRequirements, ExactPayload>;

    /// Type alias for payment requirements with Tron-specific types.
    pub type PaymentRequirements =
        wire::PaymentRequirements<ExactScheme, TronTokenAmount, Address, PaymentRequirementsExtra>;
}

#[cfg(test)]
#[cfg(any(feature = "facilitator", feature = "client"))]
mod tests {
    use alloy_primitives::{U256, keccak256};
    use alloy_sol_types::SolStruct;

    use super::*;

    /// Cross-checks the `TronWitness` typehash against the canonical byte
    /// string baked into the deployed Tron `x402ExactPermit2Proxy` contract.
    /// Tron's proxy declares its witness struct as `TronWitness` (distinct
    /// from the EVM proxy's `Witness`), so the typehash differs accordingly.
    #[test]
    fn witness_typehash_equals_spec_byte_string() {
        let witness = TronWitness {
            to: EvmAddress::ZERO,
            validAfter: U256::ZERO,
        };
        let expected = keccak256(b"TronWitness(address to,uint256 validAfter)");
        assert_eq!(witness.eip712_type_hash(), expected);
    }
}
