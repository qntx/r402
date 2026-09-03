//! Payer-agnostic `PaymentInfo` hash used as EIP-3009 / Permit2 nonce.

use alloy_primitives::{Address, B256, U256, keccak256};
use alloy_sol_types::SolValue;

use super::payload::PaymentInfo;

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
/// nonce           = keccak256(abi.encode(chainId, escrow, paymentInfoHash))
/// ```
///
/// `payer` is replaced with `address(0)` so the nonce can be computed before
/// the buyer is known. uint120/uint48/uint16 occupy full 32-byte `abi.encode`
/// slots. `escrow` is the resolved commerce-payments deployment escrow.
#[must_use]
pub fn compute_payer_agnostic_payment_info_hash(
    chain_id: u64,
    info: &PaymentInfo,
    escrow: Address,
) -> B256 {
    let encoded_info = (
        payment_info_typehash(),
        info.operator,
        Address::ZERO,
        info.receiver,
        info.token,
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
    let payment_info_hash = keccak256(encoded_info);

    let outer = (U256::from(chain_id), escrow, payment_info_hash).abi_encode();
    keccak256(outer)
}

/// Interprets a 32-byte hash as `uint256` (Permit2 nonce).
#[must_use]
pub const fn hash_as_uint256(hash: B256) -> U256 {
    U256::from_be_bytes(hash.0)
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{address, hex};

    use super::*;
    use crate::auth_capture::payload::{
        AUTH_CAPTURE_ESCROW_ADDRESS, AUTH_CAPTURE_ESCROW_V1_0_ADDRESS,
        AUTH_CAPTURE_ESCROW_V1_1_ADDRESS,
    };

    fn sample_info() -> PaymentInfo {
        PaymentInfo {
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
        }
    }

    #[test]
    fn deterministic_for_fixed_inputs() {
        let info = sample_info();
        let escrow = AUTH_CAPTURE_ESCROW_ADDRESS;
        let a = compute_payer_agnostic_payment_info_hash(8453, &info, escrow);
        let b = compute_payer_agnostic_payment_info_hash(8453, &info, escrow);
        assert_eq!(a, b);
        let mut info2 = info;
        info2.payer = Address::repeat_byte(0xFF);
        assert_eq!(
            a,
            compute_payer_agnostic_payment_info_hash(8453, &info2, escrow)
        );
        info2.salt = U256::from(1_u64);
        assert_ne!(
            a,
            compute_payer_agnostic_payment_info_hash(8453, &info2, escrow)
        );
    }

    #[test]
    fn default_escrow_is_v1_1_and_differs_from_v1_0() {
        let info = sample_info();
        let v11 =
            compute_payer_agnostic_payment_info_hash(8453, &info, AUTH_CAPTURE_ESCROW_V1_1_ADDRESS);
        let v10 =
            compute_payer_agnostic_payment_info_hash(8453, &info, AUTH_CAPTURE_ESCROW_V1_0_ADDRESS);
        assert_eq!(
            compute_payer_agnostic_payment_info_hash(8453, &info, AUTH_CAPTURE_ESCROW_ADDRESS),
            v11,
            "default alias must bind v1.1, not v1.0"
        );
        assert_ne!(v11, v10);
    }

    #[test]
    fn official_nonce_vectors() {
        let info = PaymentInfo {
            operator: address!("0x1111111111111111111111111111111111111111"),
            payer: Address::repeat_byte(0xAA),
            receiver: address!("0x2222222222222222222222222222222222222222"),
            token: address!("0x3333333333333333333333333333333333333333"),
            max_amount: U256::from(1_000_000_u64),
            pre_approval_expiry: 281_474_976_710_655,
            authorization_expiry: 281_474_976_710_655,
            refund_expiry: 281_474_976_710_655,
            min_fee_bps: 0,
            max_fee_bps: 100,
            fee_receiver: address!("0x4444444444444444444444444444444444444444"),
            salt: U256::from(1_u64),
        };
        assert_eq!(
            compute_payer_agnostic_payment_info_hash(
                84532,
                &info,
                AUTH_CAPTURE_ESCROW_V1_1_ADDRESS
            ),
            B256::from(hex!(
                "341988b065a5131b3a82818eb7aba9010135f326af1af7695fce4d2bbebd0b76"
            ))
        );
        assert_eq!(
            compute_payer_agnostic_payment_info_hash(8453, &info, AUTH_CAPTURE_ESCROW_V1_1_ADDRESS),
            B256::from(hex!(
                "a393f8f76a2327a7678488b2d504bda611b7586bb3f334b255a11bb5a75e79ca"
            ))
        );
        assert_eq!(
            compute_payer_agnostic_payment_info_hash(
                84532,
                &info,
                AUTH_CAPTURE_ESCROW_V1_0_ADDRESS
            ),
            B256::from(hex!(
                "19de8ffcb747e5caadb3dda7435cf54992e87cdf0c90e5315ffa129dbb22461e"
            ))
        );
        assert_eq!(
            compute_payer_agnostic_payment_info_hash(8453, &info, AUTH_CAPTURE_ESCROW_V1_0_ADDRESS),
            B256::from(hex!(
                "198bbfeaab2f8e36302c662ae41bceaef47a1eb4bf2549cb87aa8daa7f7bb43a"
            ))
        );
    }
}
