//! Type definitions for the EIP-155 "upto" payment scheme.
//!
//! The upto scheme uses **Permit2** exclusively: the buyer signs a
//! `PermitWitnessTransferFrom` that authorises **up to** a maximum amount,
//! and the facilitator settles for any amount in `[0, max]` via
//! [`x402UptoPermit2Proxy`](super::X402_UPTO_PERMIT2_PROXY).
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

use alloy_primitives::{Address, Bytes};
#[cfg(any(feature = "facilitator", feature = "client"))]
use alloy_sol_types::sol;
pub use r402_core::scheme::UptoScheme;
use serde::{Deserialize, Serialize};

use crate::chain::TokenAmount;

/// Serde predicate: skip the `extra` witness payload when it carries no bytes.
#[inline]
fn bytes_is_empty(b: &Bytes) -> bool {
    b.is_empty()
}

/// Permit2 witness data signed by the buyer for the upto scheme.
///
/// Matches the `Witness(address to,uint256 validAfter,bytes extra)` struct
/// expected by the deployed [`x402UptoPermit2Proxy`](super::X402_UPTO_PERMIT2_PROXY)
/// contract. The `extra` field is reserved for future use and defaults to
/// empty bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UptoPermit2Witness {
    /// Destination address for funds.
    pub to: Address,
    /// Unix timestamp (seconds) — payment invalid before this time.
    pub valid_after: TokenAmount,
    /// Reserved witness payload. Defaults to empty bytes; future spec
    /// revisions may populate this with binding data.
    #[serde(default, skip_serializing_if = "bytes_is_empty")]
    pub extra: Bytes,
}

/// Permit2 authorization parameters for the upto scheme.
///
/// Maps to the `PermitWitnessTransferFrom` struct used by the Uniswap Permit2
/// contract, with the upto-specific witness layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UptoPermit2Authorization {
    /// Signer / owner address.
    pub from: Address,
    /// Token and authorized maximum amount.
    pub permitted: crate::exact::Permit2TokenPermissions,
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
    pub signature: Bytes,
    /// Authorization parameters that were signed.
    pub permit2_authorization: UptoPermit2Authorization,
}

#[cfg(any(feature = "facilitator", feature = "client"))]
sol!(
    /// EIP-712 struct for Permit2 upto witness data.
    ///
    /// Field order and typehash MUST match the deployed
    /// [`x402UptoPermit2Proxy`](super::X402_UPTO_PERMIT2_PROXY) contract
    /// (`Witness(address to,uint256 validAfter,bytes extra)`).
    #[derive(Serialize, Deserialize)]
    struct Witness {
        address to;
        uint256 validAfter;
        bytes extra;
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
    use r402_core::wire as proto_v2;

    use super::{UptoPermit2Payload, UptoScheme};
    use crate::chain::{ChecksummedAddress, TokenAmount};

    /// Typed verify request for the upto EVM scheme.
    pub type VerifyRequest = proto_v2::TypedVerifyRequest<2, PaymentPayload, PaymentRequirements>;

    /// Typed settle request (same structure as [`VerifyRequest`]).
    pub type SettleRequest = VerifyRequest;

    /// Payment payload with embedded requirements and upto Permit2 data.
    pub type PaymentPayload = proto_v2::PaymentPayload<PaymentRequirements, UptoPermit2Payload>;

    /// Payment requirements for the upto scheme.
    ///
    /// No `extra` payload: Permit2's EIP-712 domain is always
    /// `(name = "Permit2")` regardless of the underlying token, and no
    /// asset-transfer-method selector is needed (upto is Permit2-exclusive).
    pub type PaymentRequirements =
        proto_v2::PaymentRequirements<UptoScheme, TokenAmount, ChecksummedAddress, ()>;
}

#[cfg(test)]
#[cfg(any(feature = "facilitator", feature = "client"))]
mod tests {
    use alloy_primitives::{Address, U256, keccak256};
    use alloy_sol_types::SolStruct;

    use super::*;

    /// Witness typehash MUST equal the byte-string baked into the deployed
    /// proxy. Any divergence silently breaks every signature on every chain,
    /// so we cross-check the macro expansion against the authoritative
    /// specification string.
    #[test]
    fn witness_typehash_equals_spec_byte_string() {
        let witness = Witness {
            to: Address::ZERO,
            validAfter: U256::ZERO,
            extra: Bytes::new(),
        };
        let expected = keccak256(b"Witness(address to,uint256 validAfter,bytes extra)");
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
                validAfter: U256::ZERO,
                extra: Bytes::new(),
            },
        };
        let expected = keccak256(
            b"PermitWitnessTransferFrom(TokenPermissions permitted,address spender,uint256 nonce,uint256 deadline,Witness witness)TokenPermissions(address token,uint256 amount)Witness(address to,uint256 validAfter,bytes extra)",
        );
        assert_eq!(permit.eip712_type_hash(), expected);
    }
}
