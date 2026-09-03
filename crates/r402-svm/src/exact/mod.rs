//! Solana "exact" payment scheme implementation.
//!
//! SPL Token `TransferChecked` payments with compute-budget validation,
//! simulation before settlement, and fee-payer safety checks.
//!
//! # Transaction Structure
//!
//! - Index 0: `SetComputeUnitLimit`
//! - Index 1: `SetComputeUnitPrice`
//! - Index 2: `TransferChecked` (SPL Token or Token-2022)
//! - Index 3+: Memo (always, spec §3.1) and optional Lighthouse (cap 3–7)

use r402_protocol::scheme::SchemeId;

#[cfg(feature = "client")]
pub mod client;
pub mod error;
pub use error::*;
#[cfg(feature = "facilitator")]
pub mod facilitator;
pub mod payload;
pub use payload::*;
#[cfg(feature = "server")]
pub mod server;

/// Solana exact scheme identifier.
///
/// Uses CAIP-2 chain IDs (e.g., `solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp`)
/// for chain identification and embeds requirements directly in the payload.
#[derive(Debug, Clone, Copy)]
pub struct SolanaExact;

impl SchemeId for SolanaExact {
    fn namespace(&self) -> &'static str {
        "solana"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}
