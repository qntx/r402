//! Reqwest buyer: HTTP 402 → `Payment-Signature` retry.

mod retry;
mod signature;

pub use retry::{WithPayments, X402Client};
pub use signature::{parse_payment_required, payment_signature_headers};
