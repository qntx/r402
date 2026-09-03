//! x402 Payment Protocol SDK for Rust — umbrella crate.
//!
//! `default = ["evm", "http"]` opens [`evm`] and [`http`] with each crate's
//! `client` and `server` features (K17).

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
