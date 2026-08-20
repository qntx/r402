//! Ed25519 signer for Stellar transactions and Soroban auth entries.

use std::fmt::{Debug, Formatter};
use std::str::FromStr;

use ed25519_dalek::{Signer, SigningKey};
use stellar_strkey::ed25519::{PrivateKey, PublicKey};

use super::xdr::{StellarXdrError, sign_auth_entries_for_address, sign_transaction_envelope};
use stellar_xdr::{SorobanAuthorizationEntry, TransactionEnvelope};

/// Local Ed25519 signer wrapping a Stellar secret seed (`S…`).
#[derive(Clone)]
pub struct StellarSigner {
    address: String,
    public_key: [u8; 32],
    signing_key: SigningKey,
}

impl Debug for StellarSigner {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StellarSigner")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

impl StellarSigner {
    /// Constructs a signer from a Stellar secret key (`S…`).
    ///
    /// # Errors
    ///
    /// Returns [`StellarSignerError::Parse`] if the secret is not a valid
    /// Ed25519 seed.
    pub fn from_secret(secret: impl AsRef<str>) -> Result<Self, StellarSignerError> {
        let seed = PrivateKey::from_str(secret.as_ref())
            .map_err(|e| StellarSignerError::Parse(e.to_string()))?;
        let signing_key = SigningKey::from_bytes(&seed.0);
        let verifying = signing_key.verifying_key();
        let public_key = verifying.to_bytes();
        let address = PublicKey(public_key).to_string().to_string();
        Ok(Self {
            address,
            public_key,
            signing_key,
        })
    }

    /// G-account address this signer controls.
    #[must_use]
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Raw Ed25519 public key.
    #[must_use]
    pub const fn public_key_bytes(&self) -> &[u8; 32] {
        &self.public_key
    }

    /// Signs a 32-byte digest.
    #[must_use]
    pub fn sign_digest(&self, digest: &[u8; 32]) -> [u8; 64] {
        self.signing_key.sign(digest).to_bytes()
    }

    /// Signs address-credential auth entries that belong to this signer.
    ///
    /// # Errors
    ///
    /// Returns [`StellarXdrError`] when the authorization preimage cannot be
    /// encoded.
    pub fn sign_auth_entries(
        &self,
        auth: &mut [SorobanAuthorizationEntry],
        passphrase: &str,
        expiration_ledger: u32,
    ) -> Result<usize, StellarXdrError> {
        sign_auth_entries_for_address(
            auth,
            &self.address,
            passphrase,
            expiration_ledger,
            &self.public_key,
            |digest| self.sign_digest(digest),
        )
    }

    /// Signs a transaction or fee-bump envelope.
    ///
    /// # Errors
    ///
    /// Returns [`StellarXdrError`] when the envelope cannot be hashed.
    pub fn sign_envelope(
        &self,
        envelope: &mut TransactionEnvelope,
        passphrase: &str,
    ) -> Result<(), StellarXdrError> {
        sign_transaction_envelope(envelope, passphrase, &self.public_key, |digest| {
            self.sign_digest(digest)
        })
    }
}

impl AsRef<Self> for StellarSigner {
    fn as_ref(&self) -> &Self {
        self
    }
}

/// Errors constructing a [`StellarSigner`].
#[derive(Debug, thiserror::Error)]
pub enum StellarSignerError {
    /// Secret key could not be parsed.
    #[error("invalid stellar secret key: {0}")]
    Parse(String),
}
