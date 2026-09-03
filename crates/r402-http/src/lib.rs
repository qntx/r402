//! HTTP transport layer for the x402 payment protocol.
//!
//! # Feature Flags
//!
//! - `client` — reqwest-middleware buyer: 402 → `Payment-Signature` retry
//! - `server` — Axum layer / Sequential resource-server gate

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    test,
    allow(
        clippy::indexing_slicing,
        clippy::panic,
        reason = "unit tests panic on assertion failure"
    )
)]

#[cfg(any(feature = "client", feature = "server"))]
pub mod headers;

#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub mod buyer;

#[cfg(feature = "server")]
#[cfg_attr(docsrs, doc(cfg(feature = "server")))]
pub mod server;

#[cfg(any(feature = "client", feature = "server"))]
pub use headers::{
    EXTENSION_RESPONSES, PAYMENT_REQUIRED, PAYMENT_RESPONSE, PAYMENT_SIGNATURE, SIGN_IN_WITH_X,
    X402_ALLOW_HEADERS, X402_EXPOSED_HEADERS, ensure_expose_headers, merge_private, set_no_store,
};

#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub use buyer::{WithPayments, X402Client, parse_payment_required, payment_signature_headers};

#[cfg(feature = "server")]
#[cfg_attr(docsrs, doc(cfg(feature = "server")))]
pub use server::{SettlementMode, X402Middleware};

#[cfg(test)]
mod _dev_deps {
    #[cfg(not(feature = "server"))]
    use axum_core as _;
    #[cfg(not(feature = "server"))]
    use compact_str as _;
    #[cfg(not(any(feature = "client", feature = "server")))]
    use http as _;
    #[cfg(not(feature = "client"))]
    use r402_client as _;
    #[cfg(not(feature = "server"))]
    use r402_facilitator as _;
    #[cfg(not(any(feature = "client", feature = "server")))]
    use r402_protocol as _;
    #[cfg(not(feature = "server"))]
    use r402_server as _;
    #[cfg(not(feature = "client"))]
    use reqwest as _;
    #[cfg(not(feature = "client"))]
    use reqwest_middleware as _;
    #[cfg(not(any(feature = "client", feature = "server")))]
    use serde_json as _;
    use tokio as _;
    #[cfg(not(feature = "server"))]
    use tower as _;
    #[cfg(not(feature = "server"))]
    use url as _;
    use wiremock as _;
}

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-http",
            "package name must match the crate directory"
        );
    }
}
