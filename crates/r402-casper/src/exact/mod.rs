//! Casper `"exact"` payment scheme: CEP-18 `transfer_with_authorization`.

use r402_protocol::scheme::SchemeId;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
pub mod eip712;

pub mod error;
pub use error::*;

#[cfg(feature = "facilitator")]
pub mod facilitator;
#[cfg(feature = "facilitator")]
pub use facilitator::CasperExactFacilitator;

pub mod payload;
pub use payload::*;

#[cfg(feature = "server")]
pub mod server;

/// Casper exact scheme identifier.
#[derive(Debug, Clone, Copy)]
pub struct CasperExact;

impl SchemeId for CasperExact {
    fn namespace(&self) -> &'static str {
        "casper"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_identity_matches_caip2_family() {
        assert_eq!(CasperExact.namespace(), "casper");
        assert_eq!(CasperExact.scheme(), "exact");
        assert_eq!(CasperExact.caip_family(), "casper:*");
        assert_eq!(CasperExact.id(), "casper-exact");
    }
}
