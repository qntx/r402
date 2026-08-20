//! TON `"exact"` payment scheme implementation.
//!
//! The buyer signs a W5R1 `internal_signed` message that authorizes one
//! TEP-74 `jetton_transfer`. The facilitator verifies that payload against
//! `PaymentRequirements` and relays it through a Highload V3 wallet.

#![allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    clippy::missing_errors_doc,
    clippy::missing_const_for_fn,
    clippy::redundant_closure,
    clippy::option_if_let_else,
    clippy::map_unwrap_or,
    clippy::or_fun_call,
    reason = "exact client/facilitator follow the TS checklist"
)]

use r402_core::scheme::SchemeId;

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "facilitator")]
pub mod facilitator;

#[cfg(feature = "client")]
pub mod client;

#[cfg(feature = "facilitator")]
pub mod settlement_batcher;

pub mod error;
pub use error::*;

pub mod types;
pub use types::*;

/// TON exact scheme identifier.
///
/// Uses CAIP-2 chain IDs (`tvm:-239`, `tvm:-3`) and embeds
/// requirements directly in the payload.
#[derive(Debug, Clone, Copy)]
pub struct TvmExact;

impl SchemeId for TvmExact {
    fn namespace(&self) -> &'static str {
        "tvm"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}
