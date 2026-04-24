//! EIP-155 "upto" payment scheme implementation.
//!
//! The `upto` scheme authorises a transfer **up to** a maximum amount — the
//! buyer signs the upper bound, and the facilitator settles for any amount in
//! `[0, max]` based on the resource actually consumed. It is ideal for
//! usage-based pricing (LLM tokens, bandwidth, dynamic compute).
//!
//! # Protocol snapshot
//!
//! - **Transfer method**: Permit2 only (EIP-3009 requires exact amounts at
//!   signature time and is therefore incompatible).
//! - **Proxy contract**: [`X402_UPTO_PERMIT2_PROXY`] deployed at
//!   `0x4020A4f3b7b90ccA423B9fabCc0CE57C6C240002` on every supported EVM chain.
//! - **Witness**: `{to, validAfter, extra: bytes}` — the `extra` bytes are
//!   currently reserved (empty on the wire).
//! - **Phase-dependent `amount`**: see [`types`] module docs.
//! - **Zero settlement**: `actualAmount == 0` skips the on-chain transaction
//!   entirely and returns `transaction: ""` per the spec.
//!
//! # Crates
//!
//! - [`Eip155UptoFacilitator`](facilitator::Eip155UptoFacilitator) behind the
//!   `facilitator` feature.
//! - [`Eip155UptoClient`](client::Eip155UptoClient) behind the `client` feature.
//! - [`Eip155Upto::price_tag`] behind the `server` feature.
//!
//! Reference: [x402 v2 spec — upto EVM scheme][spec].
//!
//! [spec]: https://github.com/x402-foundation/x402/blob/main/specs/schemes/upto/scheme_upto_evm.md

use alloy_primitives::{Address, address};
use r402_core::scheme::{SchemeId, sealed::Sealed};

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "facilitator")]
pub mod facilitator;
#[cfg(feature = "server")]
pub mod server;
pub mod types;
pub use types::*;

/// Canonical x402 upto Permit2 proxy address.
///
/// Vanity address `0x4020...0002`, deterministically deployed via `CREATE2`
/// with the same address on every supported EVM chain. Unlike
/// [`X402_EXACT_PERMIT2_PROXY`](crate::exact::X402_EXACT_PERMIT2_PROXY), the
/// upto proxy's `settle` function accepts a runtime `amount` parameter
/// constrained by `amount <= permit.permitted.amount`, enabling usage-based
/// settlement.
///
/// Source of truth: [x402 spec — Upto EVM scheme][spec].
///
/// [spec]: https://github.com/x402-foundation/x402/blob/main/specs/schemes/upto/scheme_upto_evm.md
pub const X402_UPTO_PERMIT2_PROXY: Address = address!("0x4020A4f3b7b90ccA423B9fabCc0CE57C6C240002");

/// EIP-155 upto payment scheme identifier.
///
/// Uses CAIP-2 chain IDs (e.g., `eip155:8453`) for chain identification and
/// embeds requirements directly in the payload.
#[derive(Debug, Clone, Copy)]
pub struct Eip155Upto;

impl SchemeId for Eip155Upto {
    fn namespace(&self) -> &'static str {
        "eip155"
    }

    fn scheme(&self) -> &str {
        UptoScheme.as_ref()
    }
}

impl Sealed for Eip155Upto {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_id_namespace_and_name() {
        assert_eq!(Eip155Upto.namespace(), "eip155");
        assert_eq!(Eip155Upto.scheme(), "upto");
    }

    #[test]
    fn proxy_address_is_canonical_x402_upto() {
        // EIP-55 checksummed display form of the x402 upto Permit2 proxy.
        assert_eq!(
            X402_UPTO_PERMIT2_PROXY.to_checksum(None),
            "0x4020A4f3b7b90ccA423B9fabCc0CE57C6C240002"
        );
    }
}
