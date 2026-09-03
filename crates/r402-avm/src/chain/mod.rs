//! Algorand chain primitives, algod RPC, and facilitator provider.
//!
//! - [`AlgorandChainReference`] — CAIP-2 truncated genesis plus full `gh`
//! - [`AlgorandAddress`] — 32-byte public key, 58-char base32 on the wire
//! - [`AlgorandTokenDeployment`] — ASA metadata
//! - [`AlgorandChainProvider`] — facilitator signers + algod (feature `facilitator`)

pub mod account;
pub use account::*;

/// Canonical msgpack encoding for Algorand transactions.
pub mod codec;
pub use codec::{
    MAX_REASONABLE_FEE_PER_TXN, MAX_TRANSACTION_GROUP_SIZE, SignedTransaction, Transaction,
    TxnType, assign_group, bytes_for_signing, decode_signed, encode_signed, encode_txn,
    max_reasonable_group_fee, txid, txid_str, txn_fee,
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
