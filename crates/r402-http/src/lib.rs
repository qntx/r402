//! HTTP transport layer for the x402 payment protocol.
//!
//! # Feature Flags
//!
//! - `client` — reqwest-middleware buyer: 402 → `Payment-Signature` retry

#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(all(test, not(feature = "client")))]
use r402_client as _;
#[cfg(all(test, not(feature = "client")))]
use r402_protocol as _;
#[cfg(all(test, not(feature = "client")))]
use reqwest as _;
#[cfg(all(test, not(feature = "client")))]
use reqwest_middleware as _;
#[cfg(all(test, not(feature = "client")))]
use serde_json as _;
#[cfg(test)]
use tokio as _;
#[cfg(test)]
use wiremock as _;

pub mod headers;

#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub mod buyer;

#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub use buyer::{WithPayments, X402Client, parse_payment_required, payment_signature_headers};
pub use headers::{
    EXTENSION_RESPONSES, PAYMENT_REQUIRED, PAYMENT_RESPONSE, PAYMENT_SIGNATURE, SIGN_IN_WITH_X,
    X402_ALLOW_HEADERS, X402_EXPOSED_HEADERS, merge_private, set_no_store,
};
