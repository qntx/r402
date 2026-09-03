//! Concordium `"exact"` payment scheme implementation.
//!
//! The buyer constructs a V1 sponsored transaction (sender signature only,
//! empty sponsor slot). The facilitator verifies the nine spec rules, adds
//! its sponsor signature, broadcasts, and waits for `ConcordiumBFT` finality.

use r402_protocol::scheme::SchemeId;

#[cfg(feature = "client")]
pub mod client;

pub mod error;
pub use error::*;

#[cfg(feature = "facilitator")]
pub mod facilitator;
#[cfg(feature = "facilitator")]
pub use facilitator::ConcordiumExactFacilitator;

pub mod payload;
pub use payload::*;

#[cfg(feature = "server")]
pub mod server;

/// Concordium exact scheme identifier.
///
/// Uses CAIP-2 chain IDs (`ccd:9dd9ca4d19e9393877d2c44b70f89acb`,
/// `ccd:4221332d34e1694168c2a0c0b3fd0f27`).
#[derive(Default)]
#[allow(
    missing_copy_implementations,
    reason = "server feature stores a Vec of money parsers"
)]
pub struct ConcordiumExact {
    #[cfg(feature = "server")]
    money_parsers: Vec<server::MoneyParser>,
}

impl ConcordiumExact {
    /// Empty scheme (no custom money parsers).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Clone for ConcordiumExact {
    fn clone(&self) -> Self {
        Self {
            #[cfg(feature = "server")]
            money_parsers: self.money_parsers.clone(),
        }
    }
}

impl std::fmt::Debug for ConcordiumExact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConcordiumExact").finish_non_exhaustive()
    }
}

impl SchemeId for ConcordiumExact {
    fn namespace(&self) -> &'static str {
        "ccd"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}
