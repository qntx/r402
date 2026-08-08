//! Casper chain support for x402 payments.
//!
//! This module provides the types used to interact with the Casper Network in
//! the x402 protocol. Settlement happens through a CEP-18 token contract's
//! `transfer_with_authorization` entry point, so the chain surface here is
//! deliberately small: identifiers, addressable keys, and token metadata.
//!
//! # Key Types
//!
//! - [`CasperChainReference`] — the chain name identifying a Casper network
//! - [`Address`] — a tagged Casper addressable key (account hash or hash)
//! - [`PublicKey`] — a tagged ed25519 / secp256k1 Casper public key
//! - [`ContractPackageHash`] — the CEP-18 package hash used as the x402 asset
//! - [`CasperTokenDeployment`] — token deployment metadata (asset + decimals)
//!
//! # Casper Networks
//!
//! Casper networks are identified by chain name, which is also the value
//! bound into every signed transaction:
//!
//! - Mainnet: `casper:casper`
//! - Testnet: `casper:casper-test`

/// Core Casper chain types (addresses, references, token deployments).
pub mod types;
pub use types::*;
