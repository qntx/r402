//! Type definitions for the V2 Solana "exact" payment scheme.
//!
//! This module re-exports types from V1 and defines V2-specific wire format
//! types for SPL Token based payments on Solana.

use r402::proto::util::U64String;
use r402::proto::v2;

use crate::chain::Address;
use crate::v1_solana_exact::types::{ExactSolanaPayload, SupportedPaymentKindExtra};

pub use crate::v1_solana_exact::types::ExactScheme;

/// V2 Solana exact verify request type.
pub type VerifyRequest = v2::VerifyRequest<PaymentPayload, PaymentRequirements>;
/// V2 Solana exact settle request type (same as verify).
pub type SettleRequest = VerifyRequest;
/// V2 Solana exact payment payload type.
pub type PaymentPayload = v2::PaymentPayload<PaymentRequirements, ExactSolanaPayload>;
/// V2 Solana exact payment requirements type.
pub type PaymentRequirements =
    v2::PaymentRequirements<ExactScheme, U64String, Address, SupportedPaymentKindExtra>;
