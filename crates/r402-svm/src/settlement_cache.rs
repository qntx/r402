//! Solana settlement deduplication cache.
//!
//! This module is a thin compatibility re-export over
//! [`r402_core::cache::SettlementCache`], the chain-agnostic primitive
//! used by every r402 facilitator (EVM exact, EVM upto, SVM exact). The
//! Solana facilitator keys the cache by the base64-encoded
//! `VersionedTransaction` payload; consult the upstream module for the
//! authoritative invariants and TTL rationale.

#![cfg(feature = "facilitator")]

pub use r402_core::cache::{
    DEFAULT_SETTLEMENT_CAPACITY as SETTLEMENT_CAPACITY, DEFAULT_SETTLEMENT_TTL as SETTLEMENT_TTL,
    Duplicate, SettlementCache,
};
