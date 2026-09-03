//! Type definitions for the EIP-155 "exact" payment scheme.
//!
//! This module defines shared wire format types for EVM exact payments,
//! supporting both EIP-3009 (`transferWithAuthorization`) and Permit2
//! transfer methods. Wire format type aliases live in the [`v2`] sub-module.

use alloy_primitives::{Address, B256, Bytes, address};
#[cfg(any(feature = "facilitator", feature = "client"))]
use alloy_sol_types::sol;
use r402_protocol::payment::UnixTimestamp;
pub use r402_protocol::scheme::ExactScheme;
use serde::{Deserialize, Serialize};

use crate::asset::AssetTransferMethod;
use crate::chain::TokenAmount;
use crate::permit2::Permit2TokenPermissions;

/// Canonical x402 exact payment Permit2 proxy address.
///
/// Vanity address `0x4020...0001`, deterministically deployed via CREATE2 with the
/// same address on every supported EVM chain. The proxy validates EIP-712 witness
/// data and atomically performs `permitTransferFromWithWitness` against the canonical
/// Permit2 contract.
///
/// Source of truth: [x402 spec — Exact EVM scheme](https://github.com/x402-foundation/x402/blob/main/specs/schemes/exact/scheme_exact_evm.md)
/// and the Go reference SDK (`go/mechanisms/evm/constants.go`).
pub const X402_EXACT_PERMIT2_PROXY: Address =
    address!("0x402085c248EeA27D92E8b30b2C58ed07f9E20001");

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
    pub const fn sender(&self) -> Address {
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

/// Witness data verified on-chain by `x402ExactPermit2Proxy`.
///
/// Included in the EIP-712 signature and checked by the proxy contract.
/// The upper time bound is enforced by Permit2's `deadline` field on the outer
/// `PermitWitnessTransferFrom` struct, so only `validAfter` lives in the witness.
///
/// The on-chain typehash is hashed from `"Witness(address to,uint256 validAfter)"`,
/// matching the canonical [exact EVM scheme](https://github.com/x402-foundation/x402/blob/main/specs/schemes/exact/scheme_exact_evm.md).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Permit2Witness {
    /// Destination address for funds.
    pub to: Address,
    /// Unix timestamp — payment invalid before this time.
    pub valid_after: TokenAmount,
}

/// Permit2 authorization parameters.
///
/// Maps to the `PermitWitnessTransferFrom` struct used by the Permit2 contract.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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
    /// Field order and typehash MUST match the deployed `x402ExactPermit2Proxy`
    /// contract definition (`Witness(address to,uint256 validAfter)`).
    #[derive(Serialize, Deserialize)]
    struct Witness {
        address to;
        uint256 validAfter;
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
    use r402_protocol::payment::{
        PaymentPayload as WirePayload, PaymentRequirements as WireReqs, TypedVerifyRequest,
    };

    use super::{ExactPayload, ExactScheme, PaymentRequirementsExtra};
    use crate::chain::{ChecksummedAddress, TokenAmount};

    /// Type alias for verify requests using the exact EVM payment scheme.
    pub type VerifyRequest = TypedVerifyRequest<2, PaymentPayload, PaymentRequirements>;

    /// Type alias for settle requests (same structure as verify requests).
    pub type SettleRequest = VerifyRequest;

    /// Type alias for payment payloads with embedded requirements and EVM-specific data.
    pub type PaymentPayload = WirePayload<PaymentRequirements, ExactPayload>;

    /// Type alias for payment requirements with EVM-specific types.
    pub type PaymentRequirements =
        WireReqs<ExactScheme, TokenAmount, ChecksummedAddress, PaymentRequirementsExtra>;
}

#[cfg(test)]
#[cfg(any(feature = "facilitator", feature = "client"))]
mod tests {
    use alloy_primitives::{U256, keccak256};
    use alloy_sol_types::SolStruct;

    use super::*;

    /// Typehash for the x402 `Witness` struct MUST equal the on-chain value
    /// baked into the `x402ExactPermit2Proxy` contract. If the `sol!` macro
    /// ever emits a field in the wrong order or with the wrong type, a signed
    /// Permit2 request would produce a hash that the proxy's `ecrecover`
    /// cannot match, breaking every payment on every chain. The authoritative
    /// byte string comes from the x402 spec, not from the Rust struct — this
    /// test cross-checks the macro expansion against an independent source.
    #[test]
    fn witness_typehash_equals_spec_byte_string() {
        let witness = Witness {
            to: Address::ZERO,
            validAfter: U256::ZERO,
        };
        let expected = keccak256(b"Witness(address to,uint256 validAfter)");
        assert_eq!(witness.eip712_type_hash(), expected);
    }

    /// Same cross-check for the top-level `PermitWitnessTransferFrom` typehash.
    /// The canonical byte string is defined by the Uniswap Permit2 contract
    /// and reproduced verbatim in the x402 spec's EVM exact scheme section.
    #[test]
    fn permit_witness_transfer_from_typehash_equals_spec_byte_string() {
        let permit = PermitWitnessTransferFrom {
            permitted: TokenPermissions {
                token: Address::ZERO,
                amount: U256::ZERO,
            },
            spender: Address::ZERO,
            nonce: U256::ZERO,
            deadline: U256::ZERO,
            witness: Witness {
                to: Address::ZERO,
                validAfter: U256::ZERO,
            },
        };
        let expected = keccak256(
            b"PermitWitnessTransferFrom(TokenPermissions permitted,address spender,uint256 nonce,uint256 deadline,Witness witness)TokenPermissions(address token,uint256 amount)Witness(address to,uint256 validAfter)",
        );
        assert_eq!(permit.eip712_type_hash(), expected);
    }
}
