//! Algorand chain support for x402 payments.
//!
//! - [`AlgorandChainReference`] — CAIP-2 truncated genesis plus full `gh`
//! - [`AlgorandAddress`] — 32-byte public key, 58-char base32 on the wire
//! - [`AlgorandTokenDeployment`] — ASA metadata
//! - [`AlgorandChainProvider`] — facilitator signers + algod (feature `facilitator`)

/// Core Algorand chain types (addresses, references, token deployments).
pub mod types;
pub use types::*;

/// Canonical msgpack encoding for Algorand transactions.
pub mod codec;
pub use codec::{
    SignedTransaction, Transaction, TxnType, assign_group, decode_signed, encode_signed,
    encode_txn, txid, txid_str, txn_fee,
};

/// Ed25519 signer bound to an Algorand address.
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod signer;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use signer::*;

/// Algod REST client.
#[cfg(any(feature = "client", feature = "facilitator"))]
pub mod rpc;
#[cfg(feature = "facilitator")]
pub use rpc::wait_for_confirmation;
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use rpc::{
    AlgodClient, AlgodError, AlgodRpc, PendingTransaction, SimulateResult, SuggestedParams,
};

/// Algorand chain provider implementation.
#[cfg(feature = "facilitator")]
pub mod provider;
#[cfg(feature = "facilitator")]
pub use provider::*;
