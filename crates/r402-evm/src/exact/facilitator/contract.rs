//! Exact-scheme Solidity interfaces (ERC-3009 + exact Permit2 proxy).
//!
//! Shared ERC-20 / EIP-6492 ABIs live in [`crate::chain::contracts`].

use alloy_sol_types::sol;

sol! {
    /// Minimal ERC-3009 + ERC-20 interface for USDC-style tokens.
    ///
    /// Only the functions actually used by the facilitator are declared.
    /// Overload order matters: bytes-signature variant is `_0`, (v,r,s) variant is `_1`.
    ///
    /// References:
    /// - ERC-3009: <https://eips.ethereum.org/EIPS/eip-3009>
    /// - USDC `FiatTokenV2_2`: <https://github.com/circlefin/stablecoin-evm>
    #[allow(missing_docs, reason = "sol! generated interface")]
    #[allow(clippy::too_many_arguments, reason = "matches on-chain ABI")]
    #[derive(Debug)]
    #[sol(rpc)]
    interface IEIP3009 {
        function name() external view returns (string);
        function version() external view returns (string);
        function balanceOf(address account) external view returns (uint256);
        function authorizationState(address authorizer, bytes32 nonce) external view returns (bool);
        function transferWithAuthorization(
            address from,
            address to,
            uint256 value,
            uint256 validAfter,
            uint256 validBefore,
            bytes32 nonce,
            bytes signature
        ) external;
        function transferWithAuthorization(
            address from,
            address to,
            uint256 value,
            uint256 validAfter,
            uint256 validBefore,
            bytes32 nonce,
            uint8 v,
            bytes32 r,
            bytes32 s
        ) external;
    }
}

sol! {
    /// x402 exact payment Permit2 proxy interface.
    ///
    /// Deployed at the canonical address [`X402_EXACT_PERMIT2_PROXY`](super::super::payload::X402_EXACT_PERMIT2_PROXY)
    /// (`0x402085c248EeA27D92E8b30b2C58ed07f9E20001`). The proxy validates the EIP-712
    /// witness against the deployed typehash `Witness(address to,uint256 validAfter)` and
    /// calls through to the canonical Permit2 contract.
    ///
    /// Reference: x402 protocol specification, exact EVM scheme.
    #[allow(missing_docs, reason = "sol! generated interface")]
    #[derive(Debug)]
    #[sol(rpc)]
    interface IX402Permit2Proxy {
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
        }

        struct EIP2612Permit {
            uint256 value;
            uint256 deadline;
            bytes32 r;
            bytes32 s;
            uint8   v;
        }

        function settle(
            Permit permit,
            address owner,
            Witness witness,
            bytes signature
        ) external;

        /// Settle while atomically broadcasting an EIP-2612 `permit` first.
        function settleWithPermit(
            EIP2612Permit permit2612,
            Permit permit,
            address owner,
            Witness witness,
            bytes signature
        ) external;

        /// Returns the canonical Permit2 contract address used by this proxy.
        ///
        /// Useful for runtime integrity checks (verify the deployed address really
        /// is the x402 proxy and not an arbitrary EOA).
        function PERMIT2() external view returns (address);
    }
}
