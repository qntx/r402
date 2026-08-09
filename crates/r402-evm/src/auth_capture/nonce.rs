//! Payer-agnostic `PaymentInfo` hash used as EIP-3009 / Permit2 nonce.

use alloy_primitives::{Address, B256, U256, keccak256};
use alloy_sol_types::{SolValue, sol};

use super::types::{AUTH_CAPTURE_ESCROW_ADDRESS, PaymentInfo};

sol! {
    /// ABI-encoded `PaymentInfo` for typehash binding (matches escrow layout).
    struct PaymentInfoAbi {
        address operator;
        address payer;
        address receiver;
        address token;
        uint120 maxAmount;
        uint48 preApprovalExpiry;
        uint48 authorizationExpiry;
        uint48 refundExpiry;
        uint16 minFeeBps;
        uint16 maxFeeBps;
        address feeReceiver;
        uint256 salt;
    }
}

/// `keccak256("PaymentInfo(address operator,address payer,...")`.
fn payment_info_typehash() -> B256 {
    keccak256(
        b"PaymentInfo(address operator,address payer,address receiver,address token,uint120 maxAmount,uint48 preApprovalExpiry,uint48 authorizationExpiry,uint48 refundExpiry,uint16 minFeeBps,uint16 maxFeeBps,address feeReceiver,uint256 salt)",
    )
}

/// Computes the payer-agnostic `PaymentInfo` hash used as the signature nonce.
///
/// ```text
/// paymentInfoHash = keccak256(abi.encode(TYPEHASH, paymentInfoWithZeroPayer))
/// nonce           = keccak256(abi.encode(chainId, ESCROW, paymentInfoHash))
/// ```
#[must_use]
pub fn compute_payer_agnostic_payment_info_hash(chain_id: u64, info: &PaymentInfo) -> B256 {
    let encoded_info = (
        payment_info_typehash(),
        info.operator,
        Address::ZERO,
        info.receiver,
        info.token,
        // uint120 stored in U256 — encode as U256; Solidity abi.encode pads
        info.max_amount,
        U256::from(info.pre_approval_expiry),
        U256::from(info.authorization_expiry),
        U256::from(info.refund_expiry),
        U256::from(info.min_fee_bps),
        U256::from(info.max_fee_bps),
        info.fee_receiver,
        info.salt,
    )
        .abi_encode();
    // The tuple encode above uses full uint256 slots which matches abi.encode
    // of the typed fields (uint120/uint48/uint16 are left-padded to 32 bytes).
    let payment_info_hash = keccak256(encoded_info);

    let outer = (
        U256::from(chain_id),
        AUTH_CAPTURE_ESCROW_ADDRESS,
        payment_info_hash,
    )
        .abi_encode();
    keccak256(outer)
}

/// Interprets a 32-byte hash as `uint256` (Permit2 nonce string form).
#[must_use]
pub const fn hash_as_uint256(hash: B256) -> U256 {
    U256::from_be_bytes(hash.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_for_fixed_inputs() {
        let info = PaymentInfo {
            operator: Address::repeat_byte(0x11),
            payer: Address::repeat_byte(0x22),
            receiver: Address::repeat_byte(0x33),
            token: Address::repeat_byte(0x44),
            max_amount: U256::from(1_000_000_u64),
            pre_approval_expiry: 1_700_000_100,
            authorization_expiry: 1_700_000_200,
            refund_expiry: 1_700_000_300,
            min_fee_bps: 0,
            max_fee_bps: 1000,
            fee_receiver: Address::repeat_byte(0x55),
            salt: U256::from(0xabc_u64),
        };
        let a = compute_payer_agnostic_payment_info_hash(8453, &info);
        let b = compute_payer_agnostic_payment_info_hash(8453, &info);
        assert_eq!(a, b);
        // Payer is zeroed — changing payer must not change hash.
        let mut info2 = info;
        info2.payer = Address::repeat_byte(0xFF);
        assert_eq!(a, compute_payer_agnostic_payment_info_hash(8453, &info2));
        // Salt change must change hash.
        info2.salt = U256::from(1_u64);
        assert_ne!(a, compute_payer_agnostic_payment_info_hash(8453, &info2));
    }
}
