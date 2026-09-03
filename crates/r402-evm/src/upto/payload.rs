//! Wire types for the EIP-155 "upto" scheme (Permit2 witness payload).
//!
//! [`v2::PaymentRequirements::amount`] is phase-dependent:
//! - verify — authorised maximum (the value the buyer signed)
//! - settle — actual charge, MUST be ≤ the signed maximum

use alloy_primitives::Address;
#[cfg(any(feature = "facilitator", feature = "client"))]
use alloy_sol_types::sol;
pub use r402_protocol::scheme::UptoScheme;
use serde::{Deserialize, Serialize};

use crate::chain::{ChecksummedAddress, TokenAmount};

/// Asset transfer method for the upto scheme. Wire value is always `"permit2"`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UptoAssetTransferMethod {
    /// Uniswap Permit2 — the only method supported by `upto`.
    #[default]
    Permit2,
}

/// Permit2 witness data signed by the buyer.
///
/// Layout MUST match `Witness(address to, address facilitator, uint256 validAfter)`
/// on [`super::X402_UPTO_PERMIT2_PROXY`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UptoPermit2Witness {
    /// Destination address for funds.
    pub to: Address,
    /// Facilitator authorised to call settle. Proxy enforces `msg.sender == facilitator`.
    pub facilitator: Address,
    /// Unix timestamp (seconds) — payment invalid before this time.
    pub valid_after: TokenAmount,
}

/// Permit2 authorization parameters for the upto scheme.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UptoPermit2Authorization {
    /// Signer / owner address.
    pub from: Address,
    /// Token and authorized maximum amount.
    pub permitted: crate::permit2::Permit2TokenPermissions,
    /// Must be [`super::X402_UPTO_PERMIT2_PROXY`].
    pub spender: Address,
    /// Unique nonce (uint256 as decimal string).
    pub nonce: TokenAmount,
    /// Signature expires after this unix timestamp (uint256 as decimal string).
    pub deadline: TokenAmount,
    /// Witness data verified by the proxy contract.
    pub witness: UptoPermit2Witness,
}

/// Permit2 payment payload sent by clients for the upto scheme.
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
/// `facilitatorAddress` is required: the buyer must commit to it at sign time
/// because the proxy enforces `msg.sender == witness.facilitator`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UptoPaymentRequirementsExtra {
    /// Token name. Used by the EIP-2612 `settleWithPermit` path; optional otherwise.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Token version. Used by the EIP-2612 `settleWithPermit` path; optional otherwise.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// Asset transfer method. Always `Permit2` for the upto scheme.
    #[serde(default)]
    pub asset_transfer_method: UptoAssetTransferMethod,
    /// Facilitator address authorised to settle this payment.
    pub facilitator_address: ChecksummedAddress,
}

impl UptoPaymentRequirementsExtra {
    /// Constructs an extra blob carrying only the facilitator binding.
    #[must_use]
    pub const fn new(facilitator_address: ChecksummedAddress) -> Self {
        Self {
            name: String::new(),
            version: String::new(),
            asset_transfer_method: UptoAssetTransferMethod::Permit2,
            facilitator_address,
        }
    }

    /// Attaches EIP-712 token domain parameters used by `settleWithPermit`.
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
    /// Typehash MUST match the deployed proxy:
    /// `Witness(address to,address facilitator,uint256 validAfter)`.
    #[derive(Serialize, Deserialize)]
    struct Witness {
        address to;
        address facilitator;
        uint256 validAfter;
    }

    /// EIP-712 struct for Permit2 upto `PermitWitnessTransferFrom`.
    #[derive(Serialize, Deserialize)]
    struct PermitWitnessTransferFrom {
        TokenPermissions permitted;
        address spender;
        uint256 nonce;
        uint256 deadline;
        Witness witness;
    }

    /// EIP-712 struct for Permit2 token permissions.
    #[derive(Serialize, Deserialize)]
    struct TokenPermissions {
        address token;
        uint256 amount;
    }
);

/// Wire format type aliases for the EIP-155 upto scheme.
pub mod v2 {
    use r402_protocol::payment as wire;

    use super::{UptoPaymentRequirementsExtra, UptoPermit2Payload, UptoScheme};
    use crate::chain::{ChecksummedAddress, TokenAmount};

    /// Typed verify request for the upto EVM scheme.
    pub type VerifyRequest = wire::TypedVerifyRequest<2, PaymentPayload, PaymentRequirements>;

    /// Typed settle request (same structure as [`VerifyRequest`]).
    pub type SettleRequest = VerifyRequest;

    /// Payment payload with embedded requirements and upto Permit2 data.
    pub type PaymentPayload = wire::PaymentPayload<PaymentRequirements, UptoPermit2Payload>;

    /// Payment requirements for the upto scheme.
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
