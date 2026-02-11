//! Solana "exact" payment scheme implementation.
//!
//! This module implements the "exact" payment scheme for Solana using
//! SPL Token `TransferChecked` instructions for token transfers.
//! Both V1 (network names) and V2 (CAIP-2 chain IDs) protocol versions
//! are supported through a unified codebase.
//!
//! # Features
//!
//! - SPL Token and Token-2022 program support
//! - Compute budget instruction validation
//! - Transaction simulation before settlement
//! - Fee payer safety checks
//! - Configurable instruction allowlists/blocklists
//!
//! # Transaction Structure
//!
//! The expected transaction structure is:
//! - Index 0: `SetComputeUnitLimit` instruction
//! - Index 1: `SetComputeUnitPrice` instruction
//! - Index 2: `TransferChecked` instruction (SPL Token or Token-2022)
//! - Index 3+: Additional instructions (if allowed by configuration)
use r402::scheme::X402SchemeId;

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "facilitator")]
pub mod facilitator;

#[cfg(feature = "client")]
pub mod client;

pub mod error;
pub use error::*;

pub mod types;
pub use types::*;

/// V1 Solana exact scheme identifier.
///
/// V1 uses network names (e.g., "solana-mainnet") for chain identification.
#[derive(Debug, Clone, Copy)]
pub struct V1SolanaExact;

impl X402SchemeId for V1SolanaExact {
    fn x402_version(&self) -> u8 {
        1
    }

    fn namespace(&self) -> &'static str {
        "solana"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}

/// V2 Solana exact scheme identifier.
///
/// V2 uses CAIP-2 chain IDs (e.g., `solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp`)
/// for chain identification and embeds requirements directly in the payload.
#[derive(Debug, Clone, Copy)]
pub struct V2SolanaExact;

impl X402SchemeId for V2SolanaExact {
    fn namespace(&self) -> &'static str {
        "solana"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}
