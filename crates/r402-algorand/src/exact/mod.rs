//! Algorand `"exact"` payment scheme implementation.
//!
//! The buyer constructs an atomic group with an ASA transfer and an optional
//! 0-amount fee-payer payment. The facilitator verifies the group, signs the
//! fee-payer transaction, simulates, and broadcasts.

use r402_protocol::scheme::SchemeId;

#[cfg(feature = "client")]
pub mod client;

pub mod error;
pub use error::*;

#[cfg(feature = "facilitator")]
pub mod facilitator;
#[cfg(feature = "facilitator")]
pub use facilitator::AlgorandExactFacilitator;

pub mod payload;
pub use payload::*;

/// Default first/last-valid window in rounds (`last_round + this`).
pub const DEFAULT_VALIDITY_ROUNDS: u64 = 1_000;

/// Default number of rounds to wait for confirmation after broadcast.
pub const DEFAULT_WAIT_ROUNDS: u32 = 10;

#[cfg(feature = "server")]
pub mod server;

/// Algorand exact scheme identifier.
///
/// Uses CAIP-2 chain IDs (`algorand:wGHE2Pwdvd7S12BL5FaOP20EGYesN73k`,
/// `algorand:SGO1GKSzyE7IEPItTxCByw9x8FmnrCDe`) and embeds requirements
/// directly in the payload.
#[derive(Debug, Clone, Copy)]
pub struct AlgorandExact;

impl SchemeId for AlgorandExact {
    fn namespace(&self) -> &'static str {
        "algorand"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}
