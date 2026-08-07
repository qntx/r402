#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

// Placeholder crate: the dependencies below are reserved for the forthcoming
// MCP transport implementation. Suppress "unused extern crate" warnings.
use r402_core as _;
#[cfg(feature = "telemetry")]
use tracing as _;
