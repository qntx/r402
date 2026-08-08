#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

#[cfg(feature = "casper")]
#[cfg_attr(docsrs, doc(cfg(feature = "casper")))]
pub use r402_casper as casper;
pub use r402_core::*;
#[cfg(feature = "evm")]
#[cfg_attr(docsrs, doc(cfg(feature = "evm")))]
pub use r402_evm as evm;
#[cfg(feature = "http")]
#[cfg_attr(docsrs, doc(cfg(feature = "http")))]
pub use r402_http as http;
#[cfg(feature = "mcp")]
#[cfg_attr(docsrs, doc(cfg(feature = "mcp")))]
pub use r402_mcp as mcp;
#[cfg(feature = "svm")]
#[cfg_attr(docsrs, doc(cfg(feature = "svm")))]
pub use r402_svm as svm;
#[cfg(feature = "tron")]
#[cfg_attr(docsrs, doc(cfg(feature = "tron")))]
pub use r402_tron as tron;
