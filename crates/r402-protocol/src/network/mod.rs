//! CAIP-2 network identifiers and chain-provider metadata.

mod id;
mod pattern;

use std::sync::Arc;

pub use id::{ChainId, ChainIdFormatError, NetworkInfo};
pub use pattern::ChainIdPattern;

/// Amount in the token's smallest unit plus the deployment it refers to.
#[derive(Debug, Clone)]
pub struct DeployedTokenAmount<TAmount, TToken> {
    /// Amount in the token's smallest unit (wei, lamports, …).
    pub amount: TAmount,
    /// Token deployment (chain, address, decimals).
    pub token: TToken,
}

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
