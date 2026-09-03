//! Alloy signer abstraction used by every EVM scheme client.

use std::future::Future;
use std::sync::Arc;

use alloy_consensus::TxEip1559;
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

    /// Whether this signer can produce EIP-1559 transaction signatures.
    ///
    /// ERC-20 approval gas sponsoring no-ops when this is `false`.
    fn signs_eip1559(&self) -> bool {
        false
    }

    /// Signs an EIP-1559 transaction. `Ok(None)` means the signer cannot sign txs.
    fn sign_eip1559(
        &self,
        tx: &mut TxEip1559,
    ) -> impl Future<Output = Result<Option<Signature>, alloy_signer::Error>> + Send {
        async {
            let _ = tx;
            Ok(None)
        }
    }
}

impl SignerLike for PrivateKeySigner {
    fn address(&self) -> Address {
        Self::address(self)
    }

    async fn sign_hash(&self, hash: &FixedBytes<32>) -> Result<Signature, alloy_signer::Error> {
        alloy_signer::Signer::sign_hash(self, hash).await
    }

    fn signs_eip1559(&self) -> bool {
        true
    }

    async fn sign_eip1559(
        &self,
        tx: &mut TxEip1559,
    ) -> Result<Option<Signature>, alloy_signer::Error> {
        let sig = alloy_network::TxSigner::sign_transaction(self, tx).await?;
        Ok(Some(sig))
    }
}

impl<T: SignerLike + Send + Sync> SignerLike for Arc<T> {
    fn address(&self) -> Address {
        (**self).address()
    }

    async fn sign_hash(&self, hash: &FixedBytes<32>) -> Result<Signature, alloy_signer::Error> {
        (**self).sign_hash(hash).await
    }

    fn signs_eip1559(&self) -> bool {
        (**self).signs_eip1559()
    }

    async fn sign_eip1559(
        &self,
        tx: &mut TxEip1559,
    ) -> Result<Option<Signature>, alloy_signer::Error> {
        (**self).sign_eip1559(tx).await
    }
}
