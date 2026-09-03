//! Casper addressable keys and public keys.
//!
//! `payTo` / `authorization.from` / `authorization.to` are tagged addressable
//! keys (`00` account hash, `01` hash). `publicKey` is a tagged algorithm
//! key (`01` ed25519, `02` secp256k1). Account hashes are
//! `blake2b-256(algorithm_name || 0x00 || key_body)`.

use std::fmt::{self, Debug, Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::codec::{self, HASH_HEX_LEN, HexDecodeError};

/// Errors produced while parsing a Casper address.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AddressParseError {
    /// The value was not 66 hex characters (2-char tag + 32-byte hash).
    #[error("casper address must be 66 hex characters, got {0}")]
    InvalidLength(usize),
    /// The leading key tag was neither `00` (account hash) nor `01` (hash).
    #[error("unsupported casper key tag {0:?}, expected 00 (account) or 01 (hash)")]
    InvalidTag(String),
    /// The value contained non-hex characters.
    #[error("casper address is not valid hex: {0}")]
    NotHex(#[from] HexDecodeError),
}

/// The kind of key an [`Address`] wraps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AddressKind {
    /// `00`-tagged account hash — the blake2b hash of a public key.
    Account,
    /// `01`-tagged hash — an addressable contract or package hash.
    Hash,
}

impl AddressKind {
    /// The two-character wire tag for this kind.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Account => "00",
            Self::Hash => "01",
        }
    }
}

/// A Casper addressable key rendered as a 66-character hex string.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Address {
    kind: AddressKind,
    bytes: [u8; 32],
}

impl Address {
    /// Builds an address from its kind and 32-byte hash.
    #[must_use]
    pub const fn new(kind: AddressKind, bytes: [u8; 32]) -> Self {
        Self { kind, bytes }
    }

    /// Returns the key kind.
    #[must_use]
    pub const fn kind(&self) -> AddressKind {
        self.kind
    }

    /// Returns the 32-byte hash without its tag.
    #[must_use]
    pub const fn hash_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }

    /// Returns the full 33-byte wire form: `tag_byte ‖ hash`.
    ///
    /// Casper EIP-712 encodes addresses as `keccak256` of these bytes.
    #[must_use]
    pub fn to_tagged_bytes(self) -> [u8; 33] {
        let tag = match self.kind {
            AddressKind::Account => 0x00,
            AddressKind::Hash => 0x01,
        };
        let mut out = [tag; 33];
        for (dst, src) in out.iter_mut().skip(1).zip(self.bytes) {
            *dst = src;
        }
        out
    }

    /// Returns `true` when the address is a `00`-tagged account hash.
    #[must_use]
    pub const fn is_account(&self) -> bool {
        matches!(self.kind, AddressKind::Account)
    }

    /// Returns the untagged 64-character hash hex.
    #[must_use]
    pub fn hash_hex(&self) -> String {
        codec::encode(&self.bytes)
    }

    /// Returns `true` when `value` is a well-formed Casper address.
    #[must_use]
    pub fn is_valid(value: &str) -> bool {
        Self::from_str(value).is_ok()
    }
}

impl Debug for Address {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "Address({self})")
    }
}

impl Display for Address {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.kind.tag())?;
        f.write_str(&codec::encode(&self.bytes))
    }
}

impl FromStr for Address {
    type Err = AddressParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != HASH_HEX_LEN + 2 {
            return Err(AddressParseError::InvalidLength(s.len()));
        }
        let (tag, rest) = s.split_at(2);
        let kind = match tag {
            "00" => AddressKind::Account,
            "01" => AddressKind::Hash,
            other => return Err(AddressParseError::InvalidTag(other.to_owned())),
        };
        let bytes = codec::decode_exact::<32>(rest)?;
        Ok(Self { kind, bytes })
    }
}

impl Serialize for Address {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Errors produced while parsing a Casper public key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PublicKeyParseError {
    /// The algorithm tag was neither `01` (ed25519) nor `02` (secp256k1).
    #[error("unsupported casper public key tag {0:?}, expected 01 (ed25519) or 02 (secp256k1)")]
    InvalidTag(String),
    /// The key length did not match the algorithm implied by the tag.
    #[error("casper {algorithm} public key must be {expected} hex characters, got {actual}")]
    InvalidLength {
        /// Algorithm named by the tag.
        algorithm: &'static str,
        /// Required hex length including the tag.
        expected: usize,
        /// Observed hex length.
        actual: usize,
    },
    /// The value contained non-hex characters.
    #[error("casper public key is not valid hex: {0}")]
    NotHex(#[from] HexDecodeError),
}

/// Signature algorithm of a Casper public key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyAlgorithm {
    /// `01`-tagged ed25519 key (32-byte body).
    Ed25519,
    /// `02`-tagged secp256k1 compressed key (33-byte body).
    Secp256k1,
}

impl KeyAlgorithm {
    /// The two-character wire tag for this algorithm.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Ed25519 => "01",
            Self::Secp256k1 => "02",
        }
    }

    /// Single-byte algorithm tag used on the wire for keys and signatures.
    #[must_use]
    pub const fn tag_byte(self) -> u8 {
        match self {
            Self::Ed25519 => 0x01,
            Self::Secp256k1 => 0x02,
        }
    }

    /// Human-readable algorithm name.
    ///
    /// Also the prefix string used when deriving an [`Address`] account
    /// hash from a public key (`name || 0x00 || key_body`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ed25519 => "ed25519",
            Self::Secp256k1 => "secp256k1",
        }
    }

    /// Length of the key body in bytes (excluding the tag).
    #[must_use]
    pub const fn body_len(self) -> usize {
        match self {
            Self::Ed25519 => 32,
            Self::Secp256k1 => 33,
        }
    }
}

/// A tagged Casper public key.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PublicKey {
    algorithm: KeyAlgorithm,
    body: Vec<u8>,
}

impl PublicKey {
    /// Returns the key's signature algorithm.
    #[must_use]
    pub const fn algorithm(&self) -> KeyAlgorithm {
        self.algorithm
    }

    /// Returns the raw key body without its algorithm tag.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Derives the `00`-tagged account hash address for this public key.
    #[must_use]
    pub fn account_hash(&self) -> Address {
        use blake2::Digest as _;
        use blake2::digest::consts::U32;

        type Blake2b256 = blake2::Blake2b<U32>;

        let mut hasher = Blake2b256::new();
        hasher.update(self.algorithm.name().as_bytes());
        hasher.update([0u8]);
        hasher.update(&self.body);
        let digest = hasher.finalize();
        let mut bytes = [0u8; 32];
        for (dst, src) in bytes.iter_mut().zip(digest) {
            *dst = src;
        }
        Address::new(AddressKind::Account, bytes)
    }

    /// Returns `true` when `value` is a well-formed Casper public key.
    #[must_use]
    pub fn is_valid(value: &str) -> bool {
        Self::from_str(value).is_ok()
    }
}

impl Debug for PublicKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({self})")
    }
}

impl Display for PublicKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.algorithm.tag())?;
        f.write_str(&codec::encode(&self.body))
    }
}

impl FromStr for PublicKey {
    type Err = PublicKeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() < 2 {
            return Err(PublicKeyParseError::InvalidTag(s.to_owned()));
        }
        let (tag, body) = s.split_at(2);
        let algorithm = match tag {
            "01" => KeyAlgorithm::Ed25519,
            "02" => KeyAlgorithm::Secp256k1,
            other => return Err(PublicKeyParseError::InvalidTag(other.to_owned())),
        };
        let expected = algorithm.body_len() * 2 + 2;
        if s.len() != expected {
            return Err(PublicKeyParseError::InvalidLength {
                algorithm: algorithm.name(),
                expected,
                actual: s.len(),
            });
        }
        let body = codec::decode(body)?;
        Ok(Self { algorithm, body })
    }
}

impl Serialize for PublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCOUNT_HEX: &str = "001234567890abcdef1234567890abcdef1234567890abcdef1234567890abcd";
    const PACKAGE_HEX: &str = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";

    fn account() -> String {
        format!("00{PACKAGE_HEX}")
    }

    #[test]
    fn address_parses_account_and_hash_tags() {
        let acc: Address = account().parse().unwrap();
        assert_eq!(acc.kind(), AddressKind::Account);
        assert!(acc.is_account());
        assert_eq!(acc.hash_hex(), PACKAGE_HEX);

        let hash: Address = format!("01{PACKAGE_HEX}").parse().unwrap();
        assert_eq!(hash.kind(), AddressKind::Hash);
        assert!(!hash.is_account());
    }

    #[test]
    fn address_rejects_bad_tag_length_and_hex() {
        assert!(matches!(
            format!("02{PACKAGE_HEX}").parse::<Address>().unwrap_err(),
            AddressParseError::InvalidTag(_)
        ));
        assert!(matches!(
            ACCOUNT_HEX.parse::<Address>().unwrap_err(),
            AddressParseError::InvalidLength(64)
        ));
        assert!(matches!(
            format!("00{}", "z".repeat(64))
                .parse::<Address>()
                .unwrap_err(),
            AddressParseError::NotHex(_)
        ));
        assert!(!Address::is_valid(""));
        assert!(Address::is_valid(&account()));
    }

    #[test]
    fn address_round_trips_through_serde() {
        let address: Address = account().parse().unwrap();
        let json = serde_json::to_string(&address).unwrap();
        assert_eq!(json, format!("\"{}\"", account()));
        assert_eq!(serde_json::from_str::<Address>(&json).unwrap(), address);
    }

    #[test]
    fn public_key_accepts_ed25519_and_secp256k1() {
        let ed25519: PublicKey = format!("01{PACKAGE_HEX}").parse().unwrap();
        assert_eq!(ed25519.algorithm(), KeyAlgorithm::Ed25519);
        assert_eq!(ed25519.body().len(), 32);

        let secp: PublicKey = format!("02{PACKAGE_HEX}ab").parse().unwrap();
        assert_eq!(secp.algorithm(), KeyAlgorithm::Secp256k1);
        assert_eq!(secp.body().len(), 33);
        assert_eq!(secp.to_string(), format!("02{PACKAGE_HEX}ab"));
    }

    #[test]
    fn public_key_rejects_wrong_body_length_for_tag() {
        let err = format!("02{PACKAGE_HEX}").parse::<PublicKey>().unwrap_err();
        assert!(matches!(
            err,
            PublicKeyParseError::InvalidLength {
                algorithm: "secp256k1",
                expected: 68,
                actual: 66
            }
        ));
        assert!(matches!(
            format!("01{PACKAGE_HEX}ab")
                .parse::<PublicKey>()
                .unwrap_err(),
            PublicKeyParseError::InvalidLength {
                algorithm: "ed25519",
                ..
            }
        ));
    }

    #[test]
    fn public_key_rejects_account_hash_tag() {
        assert!(matches!(
            format!("00{PACKAGE_HEX}").parse::<PublicKey>().unwrap_err(),
            PublicKeyParseError::InvalidTag(_)
        ));
        assert!(!PublicKey::is_valid("01"));
    }

    #[test]
    fn account_hash_matches_scheme_exact_casper_fixture() {
        let public_key: PublicKey =
            "020376e4f8766e4f33bcc6e20b331b5163f363dc0106063b052ad38afe08637bd867"
                .parse()
                .unwrap();
        assert_eq!(
            public_key.account_hash().to_string(),
            "0076d080b4e769f0b29c77fc6472d6e425710840c2f46a4506e5544d2ce34f43a3"
        );
    }

    #[test]
    fn tagged_bytes_prefix_matches_kind() {
        let acc: Address = account().parse().unwrap();
        let tagged = acc.to_tagged_bytes();
        assert_eq!(tagged[0], 0x00);
        assert_eq!(&tagged[1..], acc.hash_bytes());
    }
}
