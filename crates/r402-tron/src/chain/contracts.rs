//! Solidity ABI bindings for the contracts Tron facilitators call into.
//!
//! These generate [`alloy_sol_types::SolCall`] encoders/decoders (function
//! selector + calldata), used to build the `data` field of a TronGrid
//! `triggersmartcontract` request. This is distinct from the TIP-712
//! *struct* definitions in [`crate::exact::types`], which are used for
//! typed-data signing rather than calldata encoding.

use alloy_sol_types::sol;

/// Standard TRC-20 read-only calls, used to check payer balance.
pub mod trc20 {
    use super::sol;

    sol! {
        function balanceOf(address account) external view returns (uint256);
    }
}

/// EIP-3009 `transferWithAuthorization` call, for the `eip3009` transfer method.
pub mod eip3009 {
    use super::sol;

    sol! {
        function authorizationState(address authorizer, bytes32 nonce) external view returns (bool);
        function transferWithAuthorization(
            address from,
            address to,
            uint256 value,
            uint256 validAfter,
            uint256 validBefore,
            bytes32 nonce,
            bytes calldata signature
        ) external;
    }
}

/// `x402ExactPermit2Proxy.settle` and its argument structs, for the Permit2 transfer method.
pub mod x402_exact_permit2_proxy {
    use super::sol;

    sol! {
        struct TronTokenPermissions {
            address token;
            uint256 amount;
        }

        struct TronPermitTransferFrom {
            TronTokenPermissions permitted;
            uint256 nonce;
            uint256 deadline;
        }

        struct TronWitness {
            address to;
            uint256 validAfter;
        }

        function settle(
            TronPermitTransferFrom permit,
            address owner,
            TronWitness witness,
            bytes signature
        ) external;
    }
}
