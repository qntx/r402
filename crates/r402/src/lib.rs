//! x402 Payment Protocol SDK for Rust — umbrella crate.
//!
//! `default = ["evm", "http"]` opens [`evm`] and [`http`] with each crate's
//! `client` and `server` features.
//!
//! Protocol version 2 only. [`evm::Eip155Exact::price_tag`] takes three
//! arguments. [`http::server::X402Middleware::with_price_tag`] returns
//! `Result` (`BuildError::MissingBaseUrl` when `base_url` was omitted).
//! [`http::server::X402Layer`] is the Tower layer: there is no `X402Route`
//! and no `into_layer`.
//!
//! Consumer binaries typically return `anyhow::Result`.
//!
//! ```no_run
//! fn paid_router() -> Result<axum::Router, Box<dyn std::error::Error>> {
//!     use alloy_primitives::address;
//!     use axum::{Router, routing::get};
//!     use r402::evm::{Eip155Exact, USDC};
//!     use r402::http::server::{SettlementMode, X402Middleware};
//!
//!     async fn handler() -> &'static str {
//!         "ok"
//!     }
//!
//!     let x402 = X402Middleware::try_new("https://facilitator.example.com")?
//!         .with_base_url("https://api.example.com".parse()?)
//!         .with_scheme("eip155:*".parse()?, Eip155Exact);
//!
//!     let app = Router::new().route(
//!         "/paid-content",
//!         get(handler).layer(
//!             x402.with_price_tag(Eip155Exact::price_tag(
//!                 address!("0x1111111111111111111111111111111111111111"),
//!                 USDC::base().amount(1_000_000u64),
//!                 None,
//!             ))?
//!             .with_settlement_mode(SettlementMode::Sequential),
//!         ),
//!     );
//!     Ok(app)
//! }
//! ```
//!
//! [`http::server::SettlementMode`] is Sequential (default), Concurrent, or
//! Background.
//!
//! ```
//! use r402::http::server::SettlementMode;
//!
//! assert_eq!(SettlementMode::default(), SettlementMode::Sequential);
//! let _ = SettlementMode::Concurrent;
//! let _ = SettlementMode::Background;
//! ```

#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "evm")]
#[cfg_attr(docsrs, doc(cfg(feature = "evm")))]
pub use r402_evm as evm;
#[cfg(feature = "http")]
#[cfg_attr(docsrs, doc(cfg(feature = "http")))]
pub use r402_http as http;
#[cfg(feature = "http")]
pub use r402_http::SettlementMode;

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
