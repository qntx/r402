#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

#[cfg(feature = "algorand")]
#[cfg_attr(docsrs, doc(cfg(feature = "algorand")))]
pub use r402_algorand as algorand;
#[cfg(feature = "casper")]
#[cfg_attr(docsrs, doc(cfg(feature = "casper")))]
pub use r402_casper as casper;
pub use r402_core::*;
#[cfg(feature = "evm")]
#[cfg_attr(docsrs, doc(cfg(feature = "evm")))]
pub use r402_evm as evm;
#[cfg(feature = "hedera")]
#[cfg_attr(docsrs, doc(cfg(feature = "hedera")))]
pub use r402_hedera as hedera;
#[cfg(feature = "http")]
#[cfg_attr(docsrs, doc(cfg(feature = "http")))]
pub use r402_http as http;
#[cfg(feature = "mcp")]
#[cfg_attr(docsrs, doc(cfg(feature = "mcp")))]
pub use r402_mcp as mcp;
#[cfg(feature = "near")]
#[cfg_attr(docsrs, doc(cfg(feature = "near")))]
pub use r402_near as near;
#[cfg(feature = "solana")]
#[cfg_attr(docsrs, doc(cfg(feature = "solana")))]
pub use r402_solana as solana;
#[cfg(feature = "tron")]
#[cfg_attr(docsrs, doc(cfg(feature = "tron")))]
pub use r402_tron as tron;
#[cfg(feature = "xrpl")]
#[cfg_attr(docsrs, doc(cfg(feature = "xrpl")))]
pub use r402_xrpl as xrpl;
