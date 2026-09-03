//! Batch-settlement contract ABI (official `batch-settlement/abi.ts`).

use alloy_sol_types::sol;

sol! {
    /// `x402BatchSettlement` surface used by deposit / claim / settle / refund.
    #[allow(missing_docs, reason = "sol! generated interface")]
    #[derive(Debug)]
    #[sol(rpc)]
    interface IBatchSettlement {
        struct ChannelConfig {
            address payer;
            address payerAuthorizer;
            address receiver;
            address receiverAuthorizer;
            address token;
            uint40 withdrawDelay;
            bytes32 salt;
        }

        struct Voucher {
            ChannelConfig channel;
            uint128 maxClaimableAmount;
        }

        struct VoucherClaim {
            Voucher voucher;
            bytes signature;
            uint128 totalClaimed;
        }

        function deposit(
            ChannelConfig config,
            uint128 amount,
            address collector,
            bytes collectorData
        ) external;

        function claimWithSignature(
            VoucherClaim[] voucherClaims,
            bytes authorizerSignature
        ) external;

        function settle(address receiver, address token) external;

        function refundWithSignature(
            ChannelConfig config,
            uint128 amount,
            uint256 nonce,
            bytes receiverAuthorizerSignature
        ) external;

        function multicall(bytes[] data) external returns (bytes[] results);

        function channels(bytes32 channelId) external view returns (uint128 balance, uint128 totalClaimed);

        function pendingWithdrawals(bytes32 channelId) external view returns (uint128 amount, uint40 initiatedAt);

        function refundNonce(bytes32 channelId) external view returns (uint256);

        function receivers(address receiver, address token)
            external
            view
            returns (uint128 totalClaimed, uint128 totalSettled);

        event Settled(
            address indexed receiver,
            address indexed token,
            address indexed sender,
            uint128 amount
        );
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::keccak256;
    use alloy_sol_types::SolCall;

    use super::IBatchSettlement;

    fn sel(sig: &str) -> [u8; 4] {
        let hash = keccak256(sig.as_bytes());
        [hash[0], hash[1], hash[2], hash[3]]
    }

    #[test]
    fn selectors_match_canonical_signatures() {
        assert_eq!(
            IBatchSettlement::depositCall::SELECTOR,
            sel(
                "deposit((address,address,address,address,address,uint40,bytes32),uint128,address,bytes)"
            )
        );
        assert_eq!(
            IBatchSettlement::claimWithSignatureCall::SELECTOR,
            sel(
                "claimWithSignature((((address,address,address,address,address,uint40,bytes32),uint128),bytes,uint128)[],bytes)"
            )
        );
        assert_eq!(
            IBatchSettlement::settleCall::SELECTOR,
            sel("settle(address,address)")
        );
        assert_eq!(
            IBatchSettlement::refundWithSignatureCall::SELECTOR,
            sel(
                "refundWithSignature((address,address,address,address,address,uint40,bytes32),uint128,uint256,bytes)"
            )
        );
        assert_eq!(
            IBatchSettlement::multicallCall::SELECTOR,
            sel("multicall(bytes[])")
        );
    }
}
