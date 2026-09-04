//! x402 Payment Protocol SDK for Rust — umbrella crate.
//!
//! `default = ["evm", "http"]`. Other chains and MCP are feature-gated
//! re-exports of the same crates you can depend on directly. `full` enables
//! every production chain plus HTTP and MCP. `tron` and `casper` are opt-in
//! and not in `full`.
//!
//! ```no_run
//! use alloy_primitives::address;
//! use axum::{Router, routing::get};
//! use r402::evm::{Eip155Exact, USDC};
//! use r402::http::server::X402Middleware;
//!
//! # async fn handler() -> &'static str { "ok" }
//! # fn paid_router() -> Result<axum::Router, Box<dyn std::error::Error>> {
//! let x402 = X402Middleware::try_new("https://facilitator.example.com")?
//!     .with_base_url("https://api.example.com".parse()?)
//!     .with_scheme("eip155:*".parse()?, Eip155Exact);
//!
//! let app = Router::new().route(
//!     "/paid-content",
//!     get(handler).layer(
//!         x402.with_price_tag(Eip155Exact::price_tag(
//!             address!("0x1111111111111111111111111111111111111111"),
//!             USDC::base().amount(1_000_000u64),
//!             None,
//!         ))?,
//!     ),
//! );
//! # Ok(app)
//! # }
//! ```
//!
//! [`http::server::SettlementMode`] schedules the after-handler settle of
//! the `authorization` payment flow: Sequential (default), Concurrent, or
//! Background. It is not a `paymentFlow`.
//!
//! ```
//! use r402::http::server::SettlementMode;
//!
//! assert_eq!(SettlementMode::default(), SettlementMode::Sequential);
//! let _ = SettlementMode::Concurrent;
//! let _ = SettlementMode::Background;
//! ```

#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "aptos")]
#[cfg_attr(docsrs, doc(cfg(feature = "aptos")))]
pub use r402_aptos as aptos;
#[cfg(feature = "avm")]
#[cfg_attr(docsrs, doc(cfg(feature = "avm")))]
pub use r402_avm as avm;
#[cfg(feature = "casper")]
#[cfg_attr(docsrs, doc(cfg(feature = "casper")))]
pub use r402_casper as casper;
#[cfg(feature = "concordium")]
#[cfg_attr(docsrs, doc(cfg(feature = "concordium")))]
pub use r402_concordium as concordium;
#[cfg(feature = "evm")]
#[cfg_attr(docsrs, doc(cfg(feature = "evm")))]
pub use r402_evm as evm;
#[cfg(feature = "hedera")]
#[cfg_attr(docsrs, doc(cfg(feature = "hedera")))]
pub use r402_hedera as hedera;
#[cfg(feature = "http")]
#[cfg_attr(docsrs, doc(cfg(feature = "http")))]
pub use r402_http as http;
#[cfg(feature = "http")]
pub use r402_http::SettlementMode;
#[cfg(feature = "keeta")]
#[cfg_attr(docsrs, doc(cfg(feature = "keeta")))]
pub use r402_keeta as keeta;
#[cfg(feature = "mcp")]
#[cfg_attr(docsrs, doc(cfg(feature = "mcp")))]
pub use r402_mcp as mcp;
#[cfg(feature = "near")]
#[cfg_attr(docsrs, doc(cfg(feature = "near")))]
pub use r402_near as near;
pub use r402_protocol as protocol;
#[cfg(feature = "stellar")]
#[cfg_attr(docsrs, doc(cfg(feature = "stellar")))]
pub use r402_stellar as stellar;
#[cfg(feature = "svm")]
#[cfg_attr(docsrs, doc(cfg(feature = "svm")))]
pub use r402_svm as svm;
#[cfg(feature = "tron")]
#[cfg_attr(docsrs, doc(cfg(feature = "tron")))]
pub use r402_tron as tron;
#[cfg(feature = "tvm")]
#[cfg_attr(docsrs, doc(cfg(feature = "tvm")))]
pub use r402_tvm as tvm;
#[cfg(feature = "xrpl")]
#[cfg_attr(docsrs, doc(cfg(feature = "xrpl")))]
pub use r402_xrpl as xrpl;

#[cfg(test)]
mod _doc_deps {
    use alloy_primitives as _;
    use axum as _;
}

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402",
            "package name must match the crate directory"
        );
    }
}
