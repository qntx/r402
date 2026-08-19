//! Alloy signer abstraction used by every EVM scheme client.

use std::future::Future;
use std::sync::Arc;

use alloy_primitives::{Address, FixedBytes, Signature};
use alloy_signer_local::PrivateKeySigner;

/// Signing operations for owned signers and `Arc`-wrapped signers.
///
/// Alloy's `Signer` trait is not implemented for `Arc<T>`; this trait is.
pub trait SignerLike: Send + Sync {
    /// Address of the signer.
    fn address(&self) -> Address;

    /// Signs the given hash.
    fn sign_hash(
        &self,
        hash: &FixedBytes<32>,
    ) -> impl Future<Output = Result<Signature, alloy_signer::Error>> + Send;
}

impl SignerLike for PrivateKeySigner {
    fn address(&self) -> Address {
        Self::address(self)
    }

    async fn sign_hash(&self, hash: &FixedBytes<32>) -> Result<Signature, alloy_signer::Error> {
        alloy_signer::Signer::sign_hash(self, hash).await
    }
}

impl<T: SignerLike + Send + Sync> SignerLike for Arc<T> {
    fn address(&self) -> Address {
        (**self).address()
    }

    async fn sign_hash(&self, hash: &FixedBytes<32>) -> Result<Signature, alloy_signer::Error> {
        (**self).sign_hash(hash).await
    }
}
