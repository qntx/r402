//! Type definitions for the EIP-155 "upto" payment scheme.
//!
//! The upto scheme uses **Permit2** exclusively: the buyer signs a
//! `PermitWitnessTransferFrom` that authorises **up to** a maximum amount,
//! and the facilitator settles for any amount in `[0, max]` via
//! [`x402UptoPermit2Proxy`](super::X402_UPTO_PERMIT2_PROXY).
//!
//! # Witness layout
//!
//! The on-chain witness binds three immutable parameters into the EIP-712
//! signature:
//!
//! ```text
//! Witness(address to, address facilitator, uint256 validAfter)
//! ```
//!
//! The proxy contract enforces `msg.sender == witness.facilitator` at settle
//! time, so only the facilitator address the buyer signed for can claim the
//! payment.
//!
//! # Phase-dependent amount semantics
//!
//! [`v2::PaymentRequirements::amount`] is phase-dependent:
//!
//! - **verify phase** — equals the authorized maximum (the value the buyer
//!   signed over)
//! - **settle phase** — equals the actual amount to settle, which MUST be
//!   less than or equal to the maximum
//!
//! See the [x402 v2 spec — upto EVM scheme][spec] for the authoritative rules.
//!
//! [spec]: https://github.com/x402-foundation/x402/blob/main/specs/schemes/upto/scheme_upto_evm.md

use alloy_primitives::Address;
#[cfg(any(feature = "facilitator", feature = "client"))]
use alloy_sol_types::sol;
pub use r402_core::scheme::UptoScheme;
use serde::{Deserialize, Serialize};

use crate::chain::{ChecksummedAddress, TokenAmount};

/// Asset transfer method enum for the upto scheme.
///
/// The on-the-wire value is always the lowercase string `"permit2"`,
/// matching the [official spec][spec] and TypeScript / Go reference
/// implementations. Modelled as an enum to leave room for future variants
/// while keeping the wire shape closed.
///
/// [spec]: https://github.com/x402-foundation/x402/blob/main/specs/schemes/upto/scheme_upto_evm.md
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UptoAssetTransferMethod {
    /// Uniswap Permit2 — the only method supported by `upto` per spec.
    #[default]
    Permit2,
}

/// Permit2 witness data signed by the buyer for the upto scheme.
///
/// Matches the `Witness(address to,address facilitator,uint256 validAfter)`
/// struct expected by the deployed
/// [`x402UptoPermit2Proxy`](super::X402_UPTO_PERMIT2_PROXY) contract.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UptoPermit2Witness {
    /// Destination address for funds.
    pub to: Address,
    /// Facilitator address authorised to call settle on the proxy.
    /// The contract enforces `msg.sender == facilitator` and reverts with
    /// `UnauthorizedFacilitator` otherwise.
    pub facilitator: Address,
    /// Unix timestamp (seconds) — payment invalid before this time.
    pub valid_after: TokenAmount,
}

/// Permit2 authorization parameters for the upto scheme.
///
/// Maps to the `PermitWitnessTransferFrom` struct used by the Uniswap Permit2
/// contract, with the upto-specific witness layout.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UptoPermit2Authorization {
    /// Signer / owner address.
    pub from: Address,
    /// Token and authorized maximum amount.
    pub permitted: crate::permit2::Permit2TokenPermissions,
    /// Must be the [`x402UptoPermit2Proxy`](super::X402_UPTO_PERMIT2_PROXY) address.
    pub spender: Address,
    /// Unique nonce (uint256 as decimal string).
    pub nonce: TokenAmount,
    /// Signature expires after this unix timestamp (uint256 as decimal string).
    pub deadline: TokenAmount,
    /// Witness data verified by the proxy contract.
    pub witness: UptoPermit2Witness,
}

/// Permit2 payment payload sent by clients for the upto scheme.
///
/// Contains the EIP-712 signature over a `PermitWitnessTransferFrom` and the
/// authorization parameters that were signed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UptoPermit2Payload {
    /// EIP-712 signature (hex, 65 bytes for EOA, arbitrary for EIP-1271/6492).
    pub signature: alloy_primitives::Bytes,
    /// Authorization parameters that were signed.
    pub permit2_authorization: UptoPermit2Authorization,
}

/// Extra payment-requirements data for the EVM upto scheme.
///
/// Mirrors the official spec's `extra` blob (`name`, `version`,
/// `assetTransferMethod`, `facilitatorAddress`). The facilitator address is
/// **required** because the on-chain proxy enforces
/// `msg.sender == witness.facilitator` and the buyer must commit to it at
/// signature time.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UptoPaymentRequirementsExtra {
    /// Token name. Used by the EIP-2612 `settleWithPermit` extension path;
    /// optional otherwise.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Token version. Used by the EIP-2612 `settleWithPermit` extension path;
    /// optional otherwise.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// Asset transfer method. Always `Permit2` for the upto scheme.
    #[serde(default)]
    pub asset_transfer_method: UptoAssetTransferMethod,
    /// Facilitator address authorised to settle this payment. MUST equal
    /// the address embedded in `witness.facilitator` and MUST be one of the
    /// signer addresses listed in the facilitator's `/supported` response.
    pub facilitator_address: ChecksummedAddress,
}

impl UptoPaymentRequirementsExtra {
    /// Constructs an extra blob carrying only the facilitator binding.
    ///
    /// The token `name` and `version` default to empty; populate them via
    /// [`Self::with_token_eip712`] for EIP-2612 gas sponsoring support.
    #[must_use]
    pub const fn new(facilitator_address: ChecksummedAddress) -> Self {
        Self {
            name: String::new(),
            version: String::new(),
            asset_transfer_method: UptoAssetTransferMethod::Permit2,
            facilitator_address,
        }
    }

    /// Attaches EIP-712 token domain parameters used by the `settleWithPermit`
    /// extension path.
    #[must_use]
    pub fn with_token_eip712(mut self, name: String, version: String) -> Self {
        self.name = name;
        self.version = version;
        self
    }
}

#[cfg(any(feature = "facilitator", feature = "client"))]
sol!(
    /// EIP-712 struct for Permit2 upto witness data.
    ///
    /// Field order and typehash MUST match the deployed
    /// [`x402UptoPermit2Proxy`](super::X402_UPTO_PERMIT2_PROXY) contract:
    /// `Witness(address to,address facilitator,uint256 validAfter)`.
    #[derive(Serialize, Deserialize)]
    struct Witness {
        address to;
        address facilitator;
        uint256 validAfter;
    }

    /// EIP-712 struct for Permit2 upto `PermitWitnessTransferFrom`.
    ///
    /// The top-level type signed by the payer. Domain uses `name = "Permit2"`,
    /// no version, and `verifyingContract = PERMIT2_ADDRESS`.
    #[derive(Serialize, Deserialize)]
    struct PermitWitnessTransferFrom {
        TokenPermissions permitted;
        address spender;
        uint256 nonce;
        uint256 deadline;
        Witness witness;
    }

    /// EIP-712 struct for Permit2 token permissions (shared with exact scheme shape).
    #[derive(Serialize, Deserialize)]
    struct TokenPermissions {
        address token;
        uint256 amount;
    }
);

/// Wire format type aliases for the EIP-155 upto scheme.
pub mod v2 {
    use r402_core::wire;

    use super::{UptoPermit2Payload, UptoScheme};
    use crate::chain::{ChecksummedAddress, TokenAmount};

    /// Typed verify request for the upto EVM scheme.
    pub type VerifyRequest = wire::TypedVerifyRequest<2, PaymentPayload, PaymentRequirements>;

    /// Typed settle request (same structure as [`VerifyRequest`]).
    pub type SettleRequest = VerifyRequest;

    /// Payment payload with embedded requirements and upto Permit2 data.
    pub type PaymentPayload = wire::PaymentPayload<PaymentRequirements, UptoPermit2Payload>;

    use super::UptoPaymentRequirementsExtra;

    /// Payment requirements for the upto scheme.
    ///
    /// `extra` carries the facilitator address (required for client
    /// signing) plus optional token EIP-712 domain parameters used by the
    /// `settleWithPermit` gas-sponsoring extension.
    pub type PaymentRequirements = wire::PaymentRequirements<
        UptoScheme,
        TokenAmount,
        ChecksummedAddress,
        UptoPaymentRequirementsExtra,
    >;
}

#[cfg(test)]
#[cfg(any(feature = "facilitator", feature = "client"))]
mod tests {
    use alloy_primitives::{Address, U256, keccak256};
    use alloy_sol_types::SolStruct;

    use super::*;

    /// Witness typehash MUST equal the byte-string baked into the deployed
    /// proxy (`@x402/contracts/evm/src/x402UptoPermit2Proxy.sol`). Any
    /// divergence silently breaks every signature on every chain, so we
    /// cross-check the macro expansion against the authoritative
    /// specification string.
    #[test]
    fn witness_typehash_equals_spec_byte_string() {
        let witness = Witness {
            to: Address::ZERO,
            facilitator: Address::ZERO,
            validAfter: U256::ZERO,
        };
        let expected = keccak256(b"Witness(address to,address facilitator,uint256 validAfter)");
        assert_eq!(witness.eip712_type_hash(), expected);
    }

    /// Same cross-check for the outer `PermitWitnessTransferFrom` typehash.
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
                facilitator: Address::ZERO,
                validAfter: U256::ZERO,
            },
        };
        let expected = keccak256(
            b"PermitWitnessTransferFrom(TokenPermissions permitted,address spender,uint256 nonce,uint256 deadline,Witness witness)TokenPermissions(address token,uint256 amount)Witness(address to,address facilitator,uint256 validAfter)",
        );
        assert_eq!(permit.eip712_type_hash(), expected);
    }

    #[test]
    fn upto_extra_serialises_with_camel_case() {
        let extra =
            UptoPaymentRequirementsExtra::new(ChecksummedAddress::from(Address::repeat_byte(0xAA)))
                .with_token_eip712("USDC".into(), "2".into());
        let json = serde_json::to_value(&extra).unwrap();
        let obj = json.as_object().expect("extra serialises to a JSON object");
        assert_eq!(
            obj.get("name").and_then(serde_json::Value::as_str),
            Some("USDC")
        );
        assert_eq!(
            obj.get("version").and_then(serde_json::Value::as_str),
            Some("2")
        );
        assert_eq!(
            obj.get("assetTransferMethod")
                .and_then(serde_json::Value::as_str),
            Some("permit2")
        );
        assert!(
            obj.get("facilitatorAddress")
                .and_then(serde_json::Value::as_str)
                .is_some()
        );
    }

    #[test]
    fn upto_extra_round_trips() {
        let extra =
            UptoPaymentRequirementsExtra::new(ChecksummedAddress::from(Address::repeat_byte(0xCC)));
        let json = serde_json::to_value(&extra).unwrap();
        let back: UptoPaymentRequirementsExtra = serde_json::from_value(json).unwrap();
        assert_eq!(back, extra);
    }
}
