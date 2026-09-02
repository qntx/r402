//! CAIP-2 network identifiers and chain-provider metadata.

mod id;
mod pattern;
mod token;

use std::sync::Arc;

pub use id::{ChainId, ChainIdFormatError, NetworkInfo};
pub use pattern::ChainIdPattern;
pub use token::DeployedTokenAmount;

/// Common operations available on all chain providers.
pub trait ChainProvider {
    /// Addresses of configured signers for this chain.
    fn signer_addresses(&self) -> Vec<String>;

    /// CAIP-2 chain identifier for this provider.
    fn chain_id(&self) -> ChainId;
}

impl<T: ChainProvider> ChainProvider for Arc<T> {
    fn signer_addresses(&self) -> Vec<String> {
        (**self).signer_addresses()
    }

    fn chain_id(&self) -> ChainId {
        (**self).chain_id()
    }
}
