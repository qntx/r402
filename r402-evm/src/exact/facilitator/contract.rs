//! Solidity interface definitions for on-chain interactions.
//!
//! Contains the minimal ABI surface needed by the facilitator:
//! - [`IEIP3009`] — ERC-3009 + ERC-20 subset for USDC-style tokens
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
