//! Solidity interface definitions for on-chain interactions.
//!
//! Contains the minimal ABI surface needed by the facilitator:
//! - [`IEIP3009`] — ERC-3009 + ERC-20 subset for USDC-style tokens
//! - [`IX402Permit2Proxy`] — x402 Permit2 proxy for settling Permit2 payments
//! - [`IERC20`] — Minimal ERC-20 interface for allowance/balance checks
//! - [`Validator6492`] — EIP-6492 universal signature validator
//! - [`Sig6492`] — ABI-decodable prefix of an EIP-6492 wrapped signature

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
    #[allow(missing_docs)]
    #[allow(clippy::too_many_arguments)]
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
    /// EIP-6492 universal signature validator interface.
    ///
    /// Reference: <https://eips.ethereum.org/EIPS/eip-6492>
    #[allow(missing_docs)]
    #[derive(Debug)]
    #[sol(rpc)]
    interface Validator6492 {
        function isValidSig(address signer, bytes32 hash, bytes calldata signature) external returns (bool);
        function isValidSigWithSideEffects(address signer, bytes32 hash, bytes calldata signature) external returns (bool);
        error ERC1271Revert(bytes error);
        error ERC6492DeployFailed(bytes error);
    }
}

sol! {
    /// Solidity-compatible struct for decoding the prefix of an EIP-6492 signature.
    #[derive(Debug)]
    struct Sig6492 {
        address factory;
        bytes   factoryCalldata;
        bytes   innerSig;
    }
}

sol! {
    /// x402 exact payment Permit2 proxy interface.
    ///
    /// Deployed at `0x4020615294c913F045dc10f0a5cdEbd86c280001`.
    /// Settles Permit2-based payments by calling through the canonical Permit2 contract.
    ///
    /// Reference: x402 protocol specification
    #[allow(missing_docs)]
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
            bytes extra;
        }

        function settle(
            Permit permit,
            address owner,
            Witness witness,
            bytes signature
        ) external;
    }
}

sol! {
    /// Minimal ERC-20 interface for allowance and balance checks.
    #[allow(missing_docs)]
    #[derive(Debug)]
    #[sol(rpc)]
    interface IERC20 {
        function balanceOf(address account) external view returns (uint256);
        function allowance(address owner, address spender) external view returns (uint256);
    }
}
