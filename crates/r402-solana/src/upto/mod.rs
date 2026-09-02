//! Solana `upto` payment scheme: on-chain payment-channel escrow.
//!
//! The client escrows the authorized ceiling in a payment-channel `open`.
//! The facilitator broadcasts that deposit before the handler. After the
//! handler (or on cancel) the server attaches a voucher and the facilitator
//! claims `settle_and_seal` + `distribute`.
//!
//! HTTP Concurrent/Background are illegal for this escrow flow.

use r402_core::scheme::{SchemeId, Sealed, UptoScheme};

pub mod error;
pub mod payment_channels;
pub mod shared;
pub mod types;
pub use error::codes;
pub use types::{
    ASSET_TRANSFER_METHOD_CHANNEL, UptoSvmPayload, VOUCHER_SIGNATURE_FIELD, extra_keys,
    is_upto_svm_payload,
};

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "facilitator")]
pub mod facilitator;
#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "client")]
pub use client::SolanaUptoClient;
#[cfg(feature = "facilitator")]
pub use facilitator::SolanaUptoFacilitator;
#[cfg(feature = "server")]
pub use server::UptoSvmScheme;

/// Solana upto scheme identifier.
#[derive(Debug, Clone, Copy)]
pub struct SolanaUpto;

impl SchemeId for SolanaUpto {
    fn namespace(&self) -> &'static str {
        "solana"
    }

    fn scheme(&self) -> &str {
        UptoScheme.as_ref()
    }
}

impl Sealed for SolanaUpto {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_id_namespace_and_name() {
        assert_eq!(SolanaUpto.namespace(), "solana");
        assert_eq!(SolanaUpto.scheme(), "upto");
    }
}
