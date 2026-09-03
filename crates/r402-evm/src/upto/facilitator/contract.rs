//! Solidity interface for the upto Permit2 proxy.

use alloy_sol_types::sol;

sol! {
    /// x402 upto-scheme Permit2 proxy interface.
    ///
    /// Deployed at [`X402_UPTO_PERMIT2_PROXY`](super::super::X402_UPTO_PERMIT2_PROXY).
    /// Field order and revert selectors match `x402UptoPermit2Proxy.sol`.
    #[allow(missing_docs, reason = "sol! generated interface")]
    #[derive(Debug)]
    #[sol(rpc)]
    interface IX402UptoPermit2Proxy {
        struct TokenPermissions {
            address token;
            uint256 amount;
        }

        struct Permit {
            TokenPermissions permitted;
            uint256 nonce;
            uint256 deadline;
        }

        struct Witness {
            address to;
            address facilitator;
            uint256 validAfter;
        }

        struct EIP2612Permit {
            uint256 value;
            uint256 deadline;
            bytes32 r;
            bytes32 s;
            uint8   v;
        }

        /// Settle for a specific `amount` where `amount <= permit.permitted.amount`.
        function settle(
            Permit permit,
            uint256 amount,
            address owner,
            Witness witness,
            bytes signature
        ) external;

        /// Settle while atomically broadcasting an EIP-2612 `permit` first.
        function settleWithPermit(
            EIP2612Permit permit2612,
            Permit permit,
            uint256 amount,
            address owner,
            Witness witness,
            bytes signature
        ) external;

        function PERMIT2() external view returns (address);

        error AmountExceedsPermitted();
        error UnauthorizedFacilitator();
        error PaymentTooEarly();
        error Permit2612AmountMismatch();
        error ReentrancyGuardReentrantCall();
        error InvalidDestination();
        error InvalidOwner();
        error InvalidPermit2Address();
    }
}
