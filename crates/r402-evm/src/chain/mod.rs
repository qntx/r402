//! EVM chain support for x402 payments via EIP-155.
//!
//! Types and providers for ERC-3009 `transferWithAuthorization` on EVM chains.
//!
//! # Key Types
//!
//! - [`Eip155ChainReference`] — numeric chain ID (e.g. `8453` for Base)
//! - [`Eip155ChainProvider`] — RPC + wallet provider
//! - [`Eip155TokenDeployment`] — token address, decimals, EIP-712 domain
//! - [`MetaTransaction`] — target, calldata, confirmations
use alloy_primitives::Bytes;

pub mod account;

/// Appends `suffix` to ABI-encoded settlement calldata (ERC-8021 builder-code).
#[must_use]
pub fn append_data_suffix(calldata: Bytes, suffix: &[u8]) -> Bytes {
    if suffix.is_empty() {
        return calldata;
    }
    let mut out = Vec::with_capacity(calldata.len() + suffix.len());
    out.extend_from_slice(calldata.as_ref());
    out.extend_from_slice(suffix);
    Bytes::from(out)
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Bytes;

    use super::append_data_suffix;

    #[test]
    fn append_data_suffix_empty_is_identity() {
        let calldata = Bytes::from_static(&[0xde, 0xad]);
        assert_eq!(append_data_suffix(calldata.clone(), &[]), calldata);
    }

    #[test]
    fn append_data_suffix_concatenates() {
        let calldata = Bytes::from_static(&[0xde, 0xad]);
        let out = append_data_suffix(calldata, &[0xbe, 0xef]);
        assert_eq!(out.as_ref(), &[0xde, 0xad, 0xbe, 0xef]);
    }
}

/// Default clock-skew tolerance (seconds) for EIP-712 time-window checks.
///
/// Aligned with the Go SDK `DefaultPermit2DeadlineBuffer` (one EVM block ≈ 6 s).
pub const EVM_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS: u64 = 6;

/// Shared Solidity interfaces used by more than one scheme facilitator.
#[cfg(feature = "facilitator")]
pub mod contracts;

/// Pending nonce management for EVM transactions.
#[cfg(feature = "facilitator")]
pub mod nonce;
/// EVM chain provider implementation.
#[cfg(feature = "facilitator")]
pub mod provider;

pub use account::*;
#[cfg(feature = "facilitator")]
pub use nonce::*;
#[cfg(feature = "facilitator")]
pub use provider::*;
