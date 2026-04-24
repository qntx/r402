//! Solidity interface for the upto Permit2 proxy.

use alloy_sol_types::sol;

sol! {
    /// x402 upto-scheme Permit2 proxy interface.
    ///
    /// Deployed at
    /// [`X402_UPTO_PERMIT2_PROXY`](super::super::X402_UPTO_PERMIT2_PROXY)
    /// (`0x4020A4f3b7b90ccA423B9fabCc0CE57C6C240002`). The proxy accepts an
    /// `amount` parameter constrained by
    /// `amount <= permit.permitted.amount`, enabling usage-based settlement
    /// where the resource server decides the actual charge at request time.
    ///
    /// Reference: x402 v2 spec, upto EVM scheme.
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
            uint256 validAfter;
            bytes   extra;
        }

        /// Settle for a specific `amount` where
        /// `amount <= permit.permitted.amount`.
        ///
        /// Reverts with `AmountExceedsPermitted` when the constraint is
        /// violated, `TooEarly` when `validAfter` is still in the future,
        /// or `ReentrancyGuardReentrantCall` on recursive invocation.
        function settle(
            Permit permit,
            uint256 amount,
            address owner,
            Witness witness,
            bytes signature
        ) external;

        /// Returns the canonical Permit2 contract address used by this proxy.
        function PERMIT2() external view returns (address);
    }
}
