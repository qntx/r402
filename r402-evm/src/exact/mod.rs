//! EIP-155 "exact" payment scheme implementation.
//!
//! This module implements the "exact" payment scheme for EVM chains using
//! ERC-3009 `transferWithAuthorization` for gasless token transfers.
//!
//! # Features
//!
//! - EIP-712 typed data signing for payment authorization
//! - EIP-6492 support for counterfactual smart wallet signatures
//! - EIP-1271 support for deployed smart wallet signatures
//! - EOA signature support with split (v, r, s) components
//! - On-chain balance verification before settlement
//!
//! # Signature Handling
//!
//! The facilitator intelligently dispatches to different `transferWithAuthorization`
//! contract functions based on the signature format provided:
//!
//! - **EOA signatures (64-65 bytes)**: Parsed as (r, s, v) components and dispatched to
//!   `transferWithAuthorization(address,address,uint256,uint256,uint256,bytes32,uint8,bytes32,bytes32)`
//!   (the standard EIP-3009 function signature).
//!
//! - **EIP-1271 signatures (any other length)**: Passed as full signature bytes to
//!   `transferWithAuthorization(address,address,uint256,uint256,uint256,bytes32,bytes)`
//!   (a non-standard variant that accepts arbitrary signature bytes for contract wallets).
//!
//! - **EIP-6492 signatures**: Detected by the 32-byte magic suffix and validated via
//!   the universal EIP-6492 validator contract before settlement.
use r402::scheme::SchemeId;

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "facilitator")]
pub mod facilitator;

#[cfg(feature = "client")]
pub mod client;

pub mod types;
pub use types::*;

/// EIP-155 exact payment scheme identifier.
///
/// Uses CAIP-2 chain IDs (e.g., `eip155:8453`) for chain identification
/// and embeds requirements directly in the payload.
#[derive(Debug, Clone, Copy)]
pub struct Eip155Exact;

impl SchemeId for Eip155Exact {
    fn namespace(&self) -> &'static str {
        "eip155"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}
