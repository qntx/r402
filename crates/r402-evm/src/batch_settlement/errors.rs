//! Official batch-settlement wire reasons (`batch-settlement/errors.ts`).

/// Unrecognized or voucher payload on a path that requires an on-chain op.
pub const INVALID_PAYLOAD_TYPE: &str = "invalid_batch_settlement_evm_payload_type";
/// EIP-6492 factory is not on the deposit allowlist (default empty).
pub const EIP6492_FACTORY_NOT_ALLOWED: &str =
    "invalid_batch_settlement_evm_eip6492_factory_not_allowed";
/// Counterfactual wallet deploy transaction reverted.
pub const SMART_WALLET_DEPLOYMENT_FAILED: &str =
    "invalid_batch_settlement_evm_smart_wallet_deployment_failed";
/// Deposit `eth_call` simulation reverted.
pub const DEPOSIT_SIMULATION_FAILED: &str =
    "invalid_batch_settlement_evm_deposit_simulation_failed";
/// Deposit broadcast or receipt status failed.
pub const DEPOSIT_TRANSACTION_FAILED: &str =
    "invalid_batch_settlement_evm_deposit_transaction_failed";
/// Claim `eth_call` simulation reverted.
pub const CLAIM_SIMULATION_FAILED: &str = "invalid_batch_settlement_evm_claim_simulation_failed";
/// Claim broadcast or receipt status failed.
pub const CLAIM_TRANSACTION_FAILED: &str = "invalid_batch_settlement_evm_claim_transaction_failed";
/// Contract `settle` `eth_call` simulation reverted.
pub const SETTLE_SIMULATION_FAILED: &str = "invalid_batch_settlement_evm_settle_simulation_failed";
/// Contract `settle` broadcast or receipt status failed.
pub const SETTLE_TRANSACTION_FAILED: &str =
    "invalid_batch_settlement_evm_settle_transaction_failed";
/// `totalClaimed <= totalSettled` for the receiver/token pair.
pub const NOTHING_TO_SETTLE: &str = "invalid_batch_settlement_evm_nothing_to_settle";
/// Refund `eth_call` simulation reverted.
pub const REFUND_SIMULATION_FAILED: &str = "invalid_batch_settlement_evm_refund_simulation_failed";
/// Refund broadcast or receipt status failed.
pub const REFUND_TRANSACTION_FAILED: &str =
    "invalid_batch_settlement_evm_refund_transaction_failed";
/// Facilitator has no authorizer key and the payload omitted the signature.
pub const AUTHORIZER_NOT_CONFIGURED: &str =
    "invalid_batch_settlement_evm_authorizer_not_configured";
/// On-chain read (`channels` / `balanceOf` / `receivers`) failed.
pub const RPC_READ_FAILED: &str = "invalid_batch_settlement_evm_rpc_read_failed";
/// Payer token balance is below the deposit amount.
pub const INSUFFICIENT_BALANCE: &str = "invalid_batch_settlement_evm_insufficient_balance";
/// EIP-3009 authorization missing on an EIP-3009 deposit.
pub const ERC3009_AUTHORIZATION_REQUIRED: &str =
    "invalid_batch_settlement_evm_erc3009_authorization_required";
/// Permit2 authorization missing on a Permit2 deposit.
pub const PERMIT2_AUTHORIZATION_REQUIRED: &str =
    "invalid_batch_settlement_evm_permit2_authorization_required";
/// Permit2 `spender` is not the Permit2 deposit collector.
pub const PERMIT2_INVALID_SPENDER: &str = "invalid_batch_settlement_evm_permit2_invalid_spender";
/// Permit2 permitted amount does not match the deposit amount.
pub const PERMIT2_AMOUNT_MISMATCH: &str = "invalid_batch_settlement_evm_permit2_amount_mismatch";
/// Permit2 deadline is in the past.
pub const PERMIT2_DEADLINE_EXPIRED: &str = "invalid_batch_settlement_evm_permit2_deadline_expired";
/// Requirements extra is missing EIP-712 `name`/`version`.
pub const MISSING_EIP712_DOMAIN: &str = "invalid_batch_settlement_evm_missing_eip712_domain";
/// ERC-3009 `validBefore` is too close to now (or past).
pub const VALID_BEFORE_EXPIRED: &str =
    "invalid_batch_settlement_evm_payload_authorization_valid_before";
/// ERC-3009 `validAfter` is in the future.
pub const VALID_AFTER_IN_FUTURE: &str =
    "invalid_batch_settlement_evm_payload_authorization_valid_after";
/// ERC-3009 `ReceiveWithAuthorization` signer is not the payer.
pub const INVALID_RECEIVE_AUTHORIZATION_SIGNATURE: &str =
    "invalid_batch_settlement_evm_receive_authorization_signature";
/// Voucher `maxClaimableAmount` exceeds channel balance + deposit.
pub const CUMULATIVE_EXCEEDS_BALANCE: &str =
    "invalid_batch_settlement_evm_cumulative_exceeds_balance";
/// Voucher `maxClaimableAmount` is not above on-chain `totalClaimed`.
pub const CUMULATIVE_BELOW_CLAIMED: &str = "invalid_batch_settlement_evm_cumulative_below_claimed";
/// Refund available balance is zero.
pub const REFUND_NO_BALANCE: &str = "invalid_batch_settlement_evm_refund_no_balance";
/// On-chain channel balance is zero (not opened).
pub const CHANNEL_NOT_FOUND: &str = "invalid_batch_settlement_evm_channel_not_found";
/// Claimed `channelId` does not match `channelConfig`.
pub const CHANNEL_ID_MISMATCH: &str = "invalid_batch_settlement_evm_channel_id_mismatch";
/// Channel token does not match payment requirements asset.
pub const TOKEN_MISMATCH: &str = "invalid_batch_settlement_evm_token_mismatch";
/// Channel receiver does not match `payTo`.
pub const RECEIVER_MISMATCH: &str = "invalid_batch_settlement_evm_receiver_mismatch";
/// Channel `receiverAuthorizer` does not match requirements extra.
pub const RECEIVER_AUTHORIZER_MISMATCH: &str =
    "invalid_batch_settlement_evm_receiver_authorizer_mismatch";
/// Channel `withdrawDelay` does not match requirements extra.
pub const WITHDRAW_DELAY_MISMATCH: &str = "invalid_batch_settlement_evm_withdraw_delay_mismatch";
/// Channel `withdrawDelay` is outside `[900, 2592000]`.
pub const WITHDRAW_DELAY_OUT_OF_RANGE: &str =
    "invalid_batch_settlement_evm_withdraw_delay_out_of_range";
/// Voucher EIP-712 signature is invalid.
pub const INVALID_VOUCHER_SIGNATURE: &str = "invalid_batch_settlement_evm_voucher_signature";
/// Permit2 typed-data signature is invalid or `from` is not the payer.
pub const PERMIT2_INVALID_SIGNATURE: &str =
    "invalid_batch_settlement_evm_permit2_invalid_signature";
/// Permit2 allowance RPC failed or allowance is below the deposit amount.
pub const PERMIT2_ALLOWANCE_REQUIRED: &str =
    "invalid_batch_settlement_evm_permit2_allowance_required";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_reason_strings() {
        assert_eq!(
            INVALID_PAYLOAD_TYPE,
            "invalid_batch_settlement_evm_payload_type"
        );
        assert_eq!(
            EIP6492_FACTORY_NOT_ALLOWED,
            "invalid_batch_settlement_evm_eip6492_factory_not_allowed"
        );
    }
}
