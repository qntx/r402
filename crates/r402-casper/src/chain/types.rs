//! Wire format types for Casper chain interactions.
//!
//! This module provides the types that handle serialisation and
//! deserialisation of Casper-specific values in the x402 protocol wire
//! format: CAIP-2 chain references, addressable keys, public keys, and
//! CEP-18 token deployments.

use std::fmt::{self, Debug, Display, Formatter};
use std::str::FromStr;

use r402_core::chain::{ChainId, DeployedTokenAmount};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::hex;
use crate::motes::{CSPR_DECIMALS, Motes, MotesParseError};

/// The CAIP-2 namespace for Casper chains.
pub const CASPER_NAMESPACE: &str = "casper";

/// Length, in hex characters, of a 32-byte Casper hash.
const HASH_HEX_LEN: usize = 64;

/// A Casper chain reference.
///
/// Casper identifies its networks by chain name rather than by a numeric id
/// or genesis hash, so the CAIP-2 reference is the chain name itself:
///
/// - Mainnet: `casper:casper`
/// - Testnet: `casper:casper-test`
///
/// The chain name is also the value embedded into every Casper transaction's
/// `chain_name` header field, which binds a signed deploy to one network.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum CasperChainReference {
    /// Casper mainnet (`casper:casper`).
    Mainnet,
    /// Casper testnet (`casper:casper-test`).
    Testnet,
}

impl CasperChainReference {
    /// Casper mainnet (`casper:casper`).
    pub const CASPER: Self = Self::Mainnet;

    /// Casper testnet (`casper:casper-test`).
    pub const CASPER_TEST: Self = Self::Testnet;

    /// All chain references with built-in support.
    pub const ALL: &'static [Self] = &[Self::Mainnet, Self::Testnet];

    /// Returns the chain name (identical to the CAIP-2 reference).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mainnet => "casper",
            Self::Testnet => "casper-test",
        }
    }

    /// Returns the `chain_name` to embed into a Casper transaction targeting
    /// this network.
    #[must_use]
    pub const fn chain_name(self) -> &'static str {
        self.as_str()
    }

    /// Returns the public JSON-RPC endpoint operated by the Casper
    /// Association for this network.
    ///
    /// Deployments are expected to override this with their own node or a
    /// commercial endpoint; it exists so the defaults are usable.
    #[must_use]
    pub const fn default_rpc_url(self) -> &'static str {
        match self {
            Self::Mainnet => "https://node.mainnet.casper.network/rpc",
            Self::Testnet => "https://node.testnet.casper.network/rpc",
        }
    }

    /// Returns `true` when this reference denotes a test network.
    #[must_use]
    pub const fn is_testnet(self) -> bool {
        matches!(self, Self::Testnet)
    }
}

impl Debug for CasperChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "CasperChainReference({})", self.as_str())
    }
}

impl Display for CasperChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CasperChainReference {
    type Err = CasperChainReferenceFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == s)
            .ok_or_else(|| CasperChainReferenceFormatError::InvalidReference(s.to_owned()))
    }
}

impl Serialize for CasperChainReference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CasperChainReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<CasperChainReference> for ChainId {
    fn from(value: CasperChainReference) -> Self {
        Self::new(CASPER_NAMESPACE, value.as_str())
    }
}

impl TryFrom<ChainId> for CasperChainReference {
    type Error = CasperChainReferenceFormatError;

    fn try_from(value: ChainId) -> Result<Self, Self::Error> {
        let (namespace, reference) = value.into_parts();
        if namespace != CASPER_NAMESPACE {
            return Err(CasperChainReferenceFormatError::InvalidNamespace(namespace));
        }
        Self::from_str(&reference)
    }
}

/// Error type for parsing Casper chain references.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CasperChainReferenceFormatError {
    /// The namespace was not `casper`.
    #[error("Invalid namespace {0}, expected casper")]
    InvalidNamespace(String),
    /// The reference did not name a known Casper network.
    #[error("Invalid casper chain reference {0}")]
    InvalidReference(String),
}

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
    NotHex(#[from] hex::HexDecodeError),
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
///
/// The x402 Casper mechanism transports `payTo` and the authorisation's
/// `from` / `to` fields as tagged addressable keys: a two-character key tag
/// followed by the 32-byte hash. `00` denotes an account hash (the blake2b
/// digest of a public key), `01` denotes a contract/package hash.
///
/// # Examples
///
/// ```
/// use r402_casper::chain::{Address, AddressKind};
///
/// let address: Address =
///     "001234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
///         .parse()
///         .unwrap();
/// assert_eq!(address.kind(), AddressKind::Account);
/// ```
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

    /// Returns `true` when the address is a `00`-tagged account hash.
    #[must_use]
    pub const fn is_account(&self) -> bool {
        matches!(self.kind, AddressKind::Account)
    }

    /// Returns the untagged 64-character hash hex.
    #[must_use]
    pub fn hash_hex(&self) -> String {
        hex::encode(&self.bytes)
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
        f.write_str(&hex::encode(&self.bytes))
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
        let bytes = hex::decode_exact::<32>(rest)?;
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
    NotHex(#[from] hex::HexDecodeError),
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

    /// Human-readable algorithm name.
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
///
/// Casper public keys are transported as an algorithm tag followed by the
/// raw key body: `01` + 32 bytes for ed25519, `02` + 33 bytes for a
/// compressed secp256k1 key. Both are accepted here; the tag determines the
/// expected body length, so a malformed key is rejected at parse time rather
/// than at signature-verification time.
///
/// # Examples
///
/// ```
/// use r402_casper::chain::{KeyAlgorithm, PublicKey};
///
/// let ed25519: PublicKey =
///     "01aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899"
///         .parse()
///         .unwrap();
/// assert_eq!(ed25519.algorithm(), KeyAlgorithm::Ed25519);
/// ```
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
        f.write_str(&hex::encode(&self.body))
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
        let body = hex::decode(body)?;
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

/// Errors produced while parsing a CEP-18 contract package hash.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ContractPackageHashParseError {
    /// The value was not exactly 64 hex characters.
    #[error("contract package hash must be 64 hex characters, got {0}")]
    InvalidLength(usize),
    /// The value contained non-hex characters.
    #[error("contract package hash is not valid hex: {0}")]
    NotHex(#[from] hex::HexDecodeError),
}

/// A CEP-18 contract package hash — the x402 `asset` identifier on Casper.
///
/// Unlike [`Address`], a package hash is transported **untagged**: 64 hex
/// characters, matching what `casper-client` and the Casper x402 SDKs emit
/// for `requirements.asset`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContractPackageHash([u8; 32]);

impl ContractPackageHash {
    /// Builds a package hash from raw bytes.
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the raw 32-byte hash.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns `true` when `value` is a well-formed package hash.
    #[must_use]
    pub fn is_valid(value: &str) -> bool {
        Self::from_str(value).is_ok()
    }
}

impl Debug for ContractPackageHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "ContractPackageHash({self})")
    }
}

impl Display for ContractPackageHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(&self.0))
    }
}

impl FromStr for ContractPackageHash {
    type Err = ContractPackageHashParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != HASH_HEX_LEN {
            return Err(ContractPackageHashParseError::InvalidLength(s.len()));
        }
        Ok(Self(hex::decode_exact::<32>(s)?))
    }
}

impl Serialize for ContractPackageHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ContractPackageHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Information about a CEP-18 token deployment on a Casper network.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CasperTokenDeployment {
    /// The Casper network this token is deployed on.
    pub chain_reference: CasperChainReference,
    /// The CEP-18 contract package hash.
    pub address: ContractPackageHash,
    /// Number of decimal places (9 for wCSPR).
    pub decimals: u8,
    /// EIP-712 domain `name` used by the token's
    /// `transfer_with_authorization` entry point.
    pub name: &'static str,
    /// EIP-712 domain `version` used by the token's
    /// `transfer_with_authorization` entry point.
    pub version: &'static str,
}

impl CasperTokenDeployment {
    /// Creates a new token deployment descriptor.
    #[must_use]
    pub const fn new(
        chain_reference: CasperChainReference,
        address: ContractPackageHash,
        decimals: u8,
        name: &'static str,
        version: &'static str,
    ) -> Self {
        Self {
            chain_reference,
            address,
            decimals,
            name,
            version,
        }
    }

    /// Pairs a raw base-unit amount with this deployment.
    #[must_use]
    pub const fn amount(&self, motes: u128) -> DeployedTokenAmount<Motes, Self> {
        DeployedTokenAmount {
            amount: Motes::new(motes),
            token: *self,
        }
    }

    /// Parses a human-readable decimal amount into base units.
    ///
    /// # Errors
    ///
    /// Returns [`MotesParseError`] when the input is malformed or carries
    /// more precision than the token's `decimals` allow. Sub-unit precision
    /// is **never** truncated.
    pub fn parse(&self, value: &str) -> Result<DeployedTokenAmount<Motes, Self>, MotesParseError> {
        if self.decimals != CSPR_DECIMALS {
            return Err(MotesParseError::SubMotePrecision {
                digits: usize::from(self.decimals),
            });
        }
        Ok(DeployedTokenAmount {
            amount: Motes::from_cspr_str(value)?,
            token: *self,
        })
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
    fn chain_reference_maps_to_caip2() {
        let chain_id: ChainId = CasperChainReference::CASPER.into();
        assert_eq!(chain_id.to_string(), "casper:casper");
        let testnet: ChainId = CasperChainReference::CASPER_TEST.into();
        assert_eq!(testnet.to_string(), "casper:casper-test");
    }

    #[test]
    fn chain_reference_round_trips_through_chain_id() {
        for reference in CasperChainReference::ALL {
            let chain_id: ChainId = (*reference).into();
            assert_eq!(
                CasperChainReference::try_from(chain_id).unwrap(),
                *reference
            );
        }
    }

    #[test]
    fn chain_reference_rejects_foreign_namespace() {
        let err = CasperChainReference::try_from(ChainId::new("eip155", "casper")).unwrap_err();
        assert!(matches!(
            err,
            CasperChainReferenceFormatError::InvalidNamespace(ref ns) if ns == "eip155"
        ));
    }

    #[test]
    fn chain_reference_rejects_unknown_network() {
        assert!("casper-dev".parse::<CasperChainReference>().is_err());
    }

    #[test]
    fn chain_reference_exposes_chain_name_and_rpc() {
        assert_eq!(CasperChainReference::CASPER.chain_name(), "casper");
        assert_eq!(
            CasperChainReference::CASPER_TEST.chain_name(),
            "casper-test"
        );
        assert!(CasperChainReference::CASPER_TEST.is_testnet());
        assert!(!CasperChainReference::CASPER.is_testnet());
        assert!(
            CasperChainReference::CASPER_TEST
                .default_rpc_url()
                .contains("testnet")
        );
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
        // A 32-byte body under the secp256k1 tag must not be accepted.
        let err = format!("02{PACKAGE_HEX}").parse::<PublicKey>().unwrap_err();
        assert!(matches!(
            err,
            PublicKeyParseError::InvalidLength {
                algorithm: "secp256k1",
                expected: 68,
                actual: 66
            }
        ));
        // ...and neither must a 33-byte body under the ed25519 tag.
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
        // `00` is the account-hash tag, never a public key tag.
        assert!(matches!(
            format!("00{PACKAGE_HEX}").parse::<PublicKey>().unwrap_err(),
            PublicKeyParseError::InvalidTag(_)
        ));
        assert!(!PublicKey::is_valid("01"));
    }

    #[test]
    fn contract_package_hash_is_untagged() {
        let hash: ContractPackageHash = PACKAGE_HEX.parse().unwrap();
        assert_eq!(hash.to_string(), PACKAGE_HEX);
        assert_eq!(hash.as_bytes().len(), 32);
        assert!(ContractPackageHash::is_valid(PACKAGE_HEX));
        // Tagged addresses are 66 chars and must be rejected here.
        assert!(!ContractPackageHash::is_valid(&account()));
    }

    #[test]
    fn deployment_parses_exact_amounts_only() {
        let deployment = CasperTokenDeployment::new(
            CasperChainReference::CASPER,
            PACKAGE_HEX.parse().unwrap(),
            CSPR_DECIMALS,
            "Wrapped CSPR",
            "1",
        );
        assert_eq!(
            deployment.parse("2.5").unwrap().amount.inner(),
            2_500_000_000
        );
        assert_eq!(deployment.amount(7).amount.inner(), 7);
        assert!(matches!(
            deployment.parse("0.0000000001").unwrap_err(),
            MotesParseError::SubMotePrecision { digits: 10 }
        ));
    }
}
