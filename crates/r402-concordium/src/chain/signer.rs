//! Ed25519 account keys bound to a Concordium address.

use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter};

use concordium_rust_sdk::base::common::types::{CredentialIndex, KeyIndex, KeyPair};
use concordium_rust_sdk::base::id::types::{AccountKeys, CredentialData, SignatureThreshold};
use concordium_rust_sdk::base::transactions::TransactionSigner;
use ed25519_dalek::SigningKey;
use r402_protocol::error::ClientError;

use super::account::ConcordiumAddress;

/// Local Concordium signer: account address + Ed25519 key at credential 0 / key 0.
#[derive(Clone, Copy)]
pub struct ConcordiumSigner {
    address: ConcordiumAddress,
    seed: [u8; 32],
}

impl Debug for ConcordiumSigner {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConcordiumSigner")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

impl ConcordiumSigner {
    /// Constructs a signer from a `Base58Check` address and a 32-byte hex seed.
    ///
    /// # Errors
    ///
    /// Invalid address, non-hex key, or key length other than 32 bytes.
    pub fn from_secret(
        address: impl AsRef<str>,
        private_key_hex: impl AsRef<str>,
    ) -> Result<Self, ClientError> {
        let address: ConcordiumAddress = address
            .as_ref()
            .parse()
            .map_err(|e| ClientError::Signing(format!("{e}")))?;
        let key_bytes = hex::decode(private_key_hex.as_ref().trim_start_matches("0x"))
            .map_err(|e| ClientError::Signing(format!("invalid ed25519 hex: {e}")))?;
        let seed: [u8; 32] = key_bytes
            .as_slice()
            .try_into()
            .map_err(|_| ClientError::Signing("ed25519 private key must be 32 bytes".to_owned()))?;
        Ok(Self { address, seed })
    }

    /// Account this signer pays from.
    #[must_use]
    pub const fn address(&self) -> ConcordiumAddress {
        self.address
    }

    /// SDK account keys (credential 0).
    ///
    /// # Panics
    ///
    /// Never: threshold `1` is a valid [`AccountThreshold`].
    #[must_use]
    pub fn keys(&self) -> AccountKeys {
        Self::keys_from_seed(self.seed)
    }

    /// Account keys for a 32-byte Ed25519 seed at credential 0 / key 0.
    ///
    /// # Panics
    ///
    /// Never: threshold `1` is a valid [`AccountThreshold`].
    #[must_use]
    pub fn keys_from_seed(seed: [u8; 32]) -> AccountKeys {
        let signing = SigningKey::from_bytes(&seed);
        let pair = KeyPair::from(signing);
        let mut keys = BTreeMap::new();
        let _ = keys.insert(KeyIndex(0), pair);
        let cred = CredentialData {
            keys,
            threshold: SignatureThreshold::ONE,
        };
        let mut creds = BTreeMap::new();
        let _ = creds.insert(CredentialIndex { index: 0 }, cred);
        AccountKeys {
            keys: creds,
            threshold: {
                #[allow(
                    clippy::expect_used,
                    reason = "1 is a valid non-zero account threshold"
                )]
                1u8.try_into().expect("1 is a valid account threshold")
            },
        }
    }
}

impl TransactionSigner for ConcordiumSigner {
    fn sign_transaction_hash(
        &self,
        hash_to_sign: &concordium_rust_sdk::base::hashes::TransactionSignHash,
    ) -> concordium_rust_sdk::base::common::types::TransactionSignature {
        self.keys().sign_transaction_hash(hash_to_sign)
    }
}

impl AsRef<Self> for ConcordiumSigner {
    fn as_ref(&self) -> &Self {
        self
    }
}
