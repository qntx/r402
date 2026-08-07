//! Sealed-trait marker preventing third-party `SchemeServer` / `SchemeClient`
//! implementations.
//!
//! x402 schemes are protocol-critical: an unsanctioned implementation would
//! fragment the network. Only crates inside this workspace (`r402-evm`,
//! `r402-svm`, future chain crates) may implement the scheme traits.
//!
//! The marker trait lives in a public but `#[doc(hidden)]` module so that
//! official sibling crates can implement it on their scheme types while
//! external crates cannot (the path `__sealed::Sealed` is documented as
//! unstable / internal).

/// Internal sealed-trait module. **Not public API.**
#[doc(hidden)]
pub mod __sealed {
    /// Internal sealed-trait marker. **Do not implement.**
    pub trait Sealed {}
}

pub use __sealed::Sealed;

impl Sealed for super::ExactScheme {}
impl Sealed for super::UptoScheme {}
