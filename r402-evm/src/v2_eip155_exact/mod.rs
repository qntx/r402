//! V2 EIP-155 "exact" payment scheme implementation.
//!
//! This module implements the "exact" payment scheme for EVM chains using
//! the V2 x402 protocol. It builds on the V1 implementation but uses
//! CAIP-2 chain identifiers instead of network names.
//!
//! # Differences from V1
//!
//! - Uses CAIP-2 chain IDs (e.g., `eip155:8453`) instead of network names
//! - Payment requirements are embedded in the payload for verification
//! - Cleaner separation between accepted requirements and authorization
//!
//! # Features
//!
//! - EIP-712 typed data signing for payment authorization
//! - EIP-6492 support for counterfactual smart wallet signatures
//! - EIP-1271 support for deployed smart wallet signatures
//! - EOA signature support with split (v, r, s) components
//! - On-chain balance verification before settlement
//!
//! # Usage
//!
//! ```ignore
//! use r402_evm::v2_eip155_exact::V2Eip155Exact;
//! use r402_evm::networks::{KnownNetworkEip155, USDC};
//!
//! // Create a price tag for 1 USDC on Base
//! let usdc = USDC::base();
//! let price = V2Eip155Exact::price_tag(
//!     "0x1234...",  // pay_to address
//!     usdc.amount(1_000_000u64.into()),  // 1 USDC
//! );
//! ```

#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "server")]
#[allow(unused_imports)]
pub use server::*;

#[cfg(feature = "facilitator")]
pub mod facilitator;
#[cfg(feature = "facilitator")]
pub use facilitator::*;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
pub use client::*;

pub mod types;
pub use types::*;

use r402::scheme::X402SchemeId;

/// V2 EIP-155 exact payment scheme identifier.
#[derive(Debug, Clone, Copy)]
pub struct V2Eip155Exact;

impl X402SchemeId for V2Eip155Exact {
    fn namespace(&self) -> &'static str {
        "eip155"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}
