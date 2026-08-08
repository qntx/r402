#![cfg_attr(docsrs, feature(doc_cfg))]

//! Casper chain support for the x402 payment protocol.
//!
//! This crate implements the x402 "exact" payment scheme for the
//! [Casper Network](https://casper.network), settling in wCSPR — a CEP-18
//! token — through the contract's `transfer_with_authorization` entry point.
//!
//! # Features
//!
//! - **CAIP-2 Addressing**: `casper:casper` (mainnet) and
//!   `casper:casper-test` (testnet)
//! - **CEP-18 Payments**: token transfers authorised by an EIP-712
//!   `TransferWithAuthorization` signature, relayed and gas-paid by the
//!   facilitator
//! - **Exact Mote Arithmetic**: CSPR has 9 decimals; every conversion is
//!   integer-only and sub-mote precision is a typed error, never a silent
//!   truncation (see [`Motes`])
//! - **Key Validation**: tagged Casper addressable keys (`00` account hash,
//!   `01` hash) and public keys (`01` ed25519, `02` secp256k1)
//! - **Facilitator Client**: `/verify`, `/settle`, and `/supported` against
//!   the hosted Casper facilitator, overridable via config or environment
//!
//! # Architecture
//!
//! The crate mirrors the layout of the sibling chain crates:
//!
//! - [`chain`] — Casper chain types (references, addresses, deployments)
//! - [`exact`] — the Casper "exact" payment scheme
//! - [`motes`] — exact 9-decimal base-unit arithmetic
//! - [`hex`] — minimal hex codec for Casper wire values
//!
//! # Feature Flags
//!
//! - `server` — server-side price tag generation
//! - `client` — client-side authorisation assembly
//! - `facilitator` — facilitator client for verify/settle/supported
//! - `telemetry` — tracing instrumentation
//!
//! # Example
//!
//! ```
//! use r402_casper::chain::Address;
//! use r402_casper::{CasperExact, WCSPR};
//!
//! # #[cfg(feature = "server")]
//! # fn main() {
//! let pay_to: Address = "00fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321"
//!     .parse()
//!     .unwrap();
//! // 1.5 wCSPR, expressed exactly in motes.
//! let tag = CasperExact::price_tag(pay_to, WCSPR::casper_test().amount(1_500_000_000));
//! assert_eq!(tag.requirements.network.to_string(), "casper:casper-test");
//! # }
//! # #[cfg(not(feature = "server"))]
//! # fn main() {}
//! ```

// The `telemetry` feature wires tracing through `r402-core`'s instrumentation;
// this crate declares the dependency so downstream feature unification works.
#[cfg(feature = "telemetry")]
use tracing as _;

// Optional `client` deps are consumed from feature-gated modules
// (`exact::client`, `exact::eip712`). Name them at the crate root so
// `unused_crate_dependencies` still sees them under feature unification.
#[cfg(feature = "client")]
use rand as _;
#[cfg(feature = "client")]
use sha3 as _;

/// Public JSON-RPC endpoint documentation for the Casper facilitator stack.
///
/// The hosted facilitator, node endpoints, and their request/response shapes
/// are documented at this address.
pub const CASPER_DOCS_URL: &str = "https://docs.cspr.cloud";

pub mod chain;
pub mod exact;
pub mod hex;
pub mod motes;

mod networks;

pub use exact::CasperExact;
#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub use exact::{CasperExactClient, CasperSigner, Eip712Domain};
#[cfg(feature = "http-client")]
#[cfg_attr(docsrs, doc(cfg(feature = "http-client")))]
pub use exact::facilitator::ReqwestTransport;
#[cfg(feature = "facilitator")]
#[cfg_attr(docsrs, doc(cfg(feature = "facilitator")))]
pub use exact::facilitator::{CasperExactFacilitator, CasperFacilitatorConfig};
pub use motes::{CSPR_DECIMALS, MOTES_PER_CSPR, Motes, MotesParseError};
pub use networks::*;
