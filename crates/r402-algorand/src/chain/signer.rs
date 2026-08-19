//! Ed25519 signer bound to an Algorand address.

use base64::Engine;
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier, VerifyingKey};

use super::codec::{SignedTransaction, Transaction, bytes_for_signing};
use super::types::AlgorandAddress;

/// Local Algorand signer (32-byte ed25519 seed).
#[derive(Clone)]
pub struct AlgorandSigner {
    signing_key: SigningKey,
    address: AlgorandAddress,
}

impl std::fmt::Debug for AlgorandSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlgorandSigner")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

impl AlgorandSigner {
    /// Constructs a signer from a 32-byte ed25519 seed.
    #[must_use]
    pub fn from_seed(seed: [u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(&seed);
        let address = AlgorandAddress::from_public_key(signing_key.verifying_key().to_bytes());
        Self {
            signing_key,
            address,
        }
    }

    /// Constructs a signer from a base64 32-byte seed or 64-byte seed||pubkey.
    ///
    /// # Errors
    ///
    /// Returns an error when the string is not valid base64 or the decoded
    /// length is not 32 or 64 bytes.
    pub fn from_base64_secret(secret: impl AsRef<str>) -> Result<Self, SignerError> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(secret.as_ref().trim())
            .map_err(|e| SignerError(e.to_string()))?;
        let seed: [u8; 32] = match bytes.len() {
            32 => bytes
                .as_slice()
                .try_into()
                .map_err(|_| SignerError("invalid seed".to_owned()))?,
            64 => bytes
                .get(..32)
                .ok_or_else(|| SignerError("invalid seed".to_owned()))?
                .try_into()
                .map_err(|_| SignerError("invalid seed".to_owned()))?,
            other => {
                return Err(SignerError(format!(
                    "AVM private key must be 32 or 64 bytes, got {other}"
                )));
            }
        };
        Ok(Self::from_seed(seed))
    }

    /// Address this signer pays (or sponsors) from.
    #[must_use]
    pub const fn address(&self) -> AlgorandAddress {
        self.address
    }

    /// Signs `txn` and returns a `{sig, txn}` envelope.
    #[must_use]
    pub fn sign(&self, txn: &Transaction) -> SignedTransaction {
        let sig = self.signing_key.sign(&bytes_for_signing(txn));
        SignedTransaction {
            sig: Some(sig.to_bytes()),
            txn: txn.clone(),
        }
    }
}

/// Failure constructing an [`AlgorandSigner`].
#[derive(Debug, thiserror::Error)]
#[error("algorand signer: {0}")]
pub struct SignerError(pub String);

/// Verifies an ed25519 signature against `txn.sender`.
#[must_use]
pub fn verify_txn_signature(txn: &Transaction, sig: &[u8; 64]) -> bool {
    let Ok(vk) = VerifyingKey::from_bytes(txn.sender.as_bytes()) else {
        return false;
    };
    let signature = Signature::from_bytes(sig);
    vk.verify(&bytes_for_signing(txn), &signature).is_ok()
}
