//! Shared Solidity interfaces used by more than one EVM scheme facilitator.

use alloy_sol_types::sol;

sol! {
    /// EIP-6492 universal signature validator interface.
    ///
    /// Reference: <https://eips.ethereum.org/EIPS/eip-6492>
    #[allow(missing_docs, reason = "sol! generated interface")]
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
    /// Minimal ERC-20 interface for allowance and balance checks.
    #[allow(missing_docs, reason = "sol! generated interface")]
    #[derive(Debug)]
    #[sol(rpc)]
    interface IERC20 {
        function balanceOf(address account) external view returns (uint256);
        function allowance(address owner, address spender) external view returns (uint256);
        event Transfer(address indexed from, address indexed to, uint256 value);
    }
}
