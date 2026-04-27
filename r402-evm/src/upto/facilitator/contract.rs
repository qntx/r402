//! Solidity interface for the upto Permit2 proxy.
//!
//! Mirrors the canonical `x402UptoPermit2Proxy` deployed at
//! `0x4020A4f3b7b90ccA423B9fabCc0CE57C6C240002`. Field order, struct layout
//! and revert selectors are kept byte-identical to the upstream Solidity
//! source (`@x402/contracts/evm/src/x402UptoPermit2Proxy.sol`) so that the
//! `eth_call` simulation and on-chain settle paths interoperate with every
//! reference implementation (TypeScript, Go).

use alloy_sol_types::sol;

sol! {
    /// x402 upto-scheme Permit2 proxy interface.
    ///
    /// Deployed at
    /// [`X402_UPTO_PERMIT2_PROXY`](super::super::X402_UPTO_PERMIT2_PROXY)
    /// (`0x4020A4f3b7b90ccA423B9fabCc0CE57C6C240002`). The proxy accepts an
    /// `amount` parameter constrained by `amount <= permit.permitted.amount`
    /// and enforces `msg.sender == witness.facilitator` so that only the
    /// facilitator the buyer signed for can claim the payment.
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

        /// Witness binding signed by the buyer. Layout is identical to the
        /// `Witness` struct emitted by `x402UptoPermit2Proxy.sol`.
        struct Witness {
            address to;
            address facilitator;
            uint256 validAfter;
        }

        /// EIP-2612 permit parameters used by the gas-sponsoring extension
        /// path (`settleWithPermit`).
        struct EIP2612Permit {
            uint256 value;
            uint256 deadline;
            bytes32 r;
            bytes32 s;
            uint8   v;
        }

        /// Settle for a specific `amount` where `amount <= permit.permitted.amount`.
        ///
        /// Reverts with `AmountExceedsPermitted` when the constraint is
        /// violated, `UnauthorizedFacilitator` when `msg.sender` does not
        /// match `witness.facilitator`, `PaymentTooEarly` when `validAfter`
        /// is still in the future, or `ReentrancyGuardReentrantCall` on
        /// recursive invocation.
        function settle(
            Permit permit,
            uint256 amount,
            address owner,
            Witness witness,
            bytes signature
        ) external;

        /// Settle while atomically broadcasting an EIP-2612 `permit` first.
        ///
        /// Used by the `eip2612GasSponsoring` extension to onboard buyers
        /// who have not pre-approved the canonical Permit2 contract. The
        /// inner `EIP2612Permit` MUST authorise the canonical Permit2
        /// address as spender for the full `permit.permitted.amount`.
        function settleWithPermit(
            EIP2612Permit permit2612,
            Permit permit,
            uint256 amount,
            address owner,
            Witness witness,
            bytes signature
        ) external;

        /// Returns the canonical Permit2 contract address used by this proxy.
        function PERMIT2() external view returns (address);

        /// Reverted by the on-chain proxy when the requested `amount`
        /// exceeds `permit.permitted.amount`.
        error AmountExceedsPermitted();
        /// Reverted by the on-chain proxy when `msg.sender` is not the
        /// authorised `witness.facilitator`.
        error UnauthorizedFacilitator();
        /// Reverted by the on-chain proxy when `block.timestamp < validAfter`.
        error PaymentTooEarly();
        /// Reverted when the embedded EIP-2612 permit value disagrees with
        /// `permit.permitted.amount`.
        error Permit2612AmountMismatch();
        /// Reverted on recursive entry into the nonReentrant settle path.
        error ReentrancyGuardReentrantCall();
        /// Reverted on `settle` recipient mismatch (defence-in-depth check).
        error InvalidDestination();
        /// Reverted on owner / payer mismatch.
        error InvalidOwner();
        /// Reverted when the configured Permit2 address is not the canonical one.
        error InvalidPermit2Address();
    }
}
