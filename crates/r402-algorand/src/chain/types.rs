//! Wire format types for Algorand chain interactions.

use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;

use data_encoding::BASE32_NOPAD;
use r402_core::amount::{MoneyAmount, MoneyAmountParseError};
use r402_core::chain::{ChainId, DeployedTokenAmount};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha512_256};

/// The CAIP-2 namespace for Algorand chains.
pub const ALGORAND_NAMESPACE: &str = "algorand";

/// `AlgoNode` public algod for mainnet.
pub const ALGORAND_MAINNET_ALGOD_URL: &str = "https://mainnet-api.algonode.cloud";

/// `AlgoNode` public algod for testnet.
pub const ALGORAND_TESTNET_ALGOD_URL: &str = "https://testnet-api.algonode.cloud";

/// Full standard-base64 mainnet genesis hash written into on-tx `gh`.
pub const ALGORAND_MAINNET_GENESIS_HASH_B64: &str = "wGHE2Pwdvd7S12BL5FaOP20EGYesN73ktiC1qzkkit8=";

/// Full standard-base64 testnet genesis hash written into on-tx `gh`.
pub const ALGORAND_TESTNET_GENESIS_HASH_B64: &str = "SGO1GKSzyE7IEPItTxCByw9x8FmnrCDexi9/cOUJOiI=";

/// CAIP-2 reference: first 32 chars of the url-safe genesis hash (mainnet).
pub const ALGORAND_MAINNET_CAIP_REF: &str = "wGHE2Pwdvd7S12BL5FaOP20EGYesN73k";

/// CAIP-2 reference: first 32 chars of the url-safe genesis hash (testnet).
pub const ALGORAND_TESTNET_CAIP_REF: &str = "SGO1GKSzyE7IEPItTxCByw9x8FmnrCDe";

/// Decoded mainnet genesis hash (32 bytes).
pub const ALGORAND_MAINNET_GENESIS_HASH: [u8; 32] = [
    0xc0, 0x61, 0xc4, 0xd8, 0xfc, 0x1d, 0xbd, 0xde, 0xd2, 0xd7, 0x60, 0x4b, 0xe4, 0x56, 0x8e, 0x3f,
    0x6d, 0x04, 0x19, 0x87, 0xac, 0x37, 0xbd, 0xe4, 0xb6, 0x20, 0xb5, 0xab, 0x39, 0x24, 0x8a, 0xdf,
];

/// Decoded testnet genesis hash (32 bytes).
pub const ALGORAND_TESTNET_GENESIS_HASH: [u8; 32] = [
    0x48, 0x63, 0xb5, 0x18, 0xa4, 0xb3, 0xc8, 0x4e, 0xc8, 0x10, 0xf2, 0x2d, 0x4f, 0x10, 0x81, 0xcb,
    0x0f, 0x71, 0xf0, 0x59, 0xa7, 0xac, 0x20, 0xde, 0xc6, 0x2f, 0x7f, 0x70, 0xe5, 0x09, 0x3a, 0x22,
];

/// Mainnet genesis ID (`gen` field).
pub const ALGORAND_MAINNET_GENESIS_ID: &str = "mainnet-v1.0";

/// Testnet genesis ID (`gen` field).
pub const ALGORAND_TESTNET_GENESIS_ID: &str = "testnet-v1.0";

/// Native ALGO / ASA amounts in base units.
pub type MicroAlgos = u64;

/// An Algorand chain reference: truncated CAIP-2 plus the full `gh` hash.
///
/// CAIP-2 uses the first 32 characters of the url-safe genesis hash. That
/// truncation is **not** the on-tx `gh` value — always write `gh` from
/// [`Self::genesis_hash`].
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AlgorandChainReference {
    /// First 32 ASCII characters of the url-safe genesis hash.
    pub caip_ref: [u8; 32],
    /// Full standard-base64 genesis hash used for the on-tx `gh` field.
    pub genesis_hash_b64: &'static str,
}

impl AlgorandChainReference {
    /// Algorand mainnet.
    pub const MAINNET: Self = Self {
        caip_ref: *b"wGHE2Pwdvd7S12BL5FaOP20EGYesN73k",
        genesis_hash_b64: ALGORAND_MAINNET_GENESIS_HASH_B64,
    };

    /// Algorand testnet.
    pub const TESTNET: Self = Self {
        caip_ref: *b"SGO1GKSzyE7IEPItTxCByw9x8FmnrCDe",
        genesis_hash_b64: ALGORAND_TESTNET_GENESIS_HASH_B64,
    };

    /// All chain references with built-in support.
    pub const ALL: &'static [Self] = &[Self::MAINNET, Self::TESTNET];

    /// Returns the CAIP-2 reference string (first 32 chars).
    #[must_use]
    #[allow(
        clippy::expect_used,
        clippy::missing_panics_doc,
        reason = "caip_ref is constructed from ASCII literals"
    )]
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.caip_ref).expect("caip_ref is ASCII")
    }

    /// Returns the 32-byte genesis hash written into `gh`.
    #[must_use]
    pub fn genesis_hash(self) -> [u8; 32] {
        if self == Self::MAINNET {
            ALGORAND_MAINNET_GENESIS_HASH
        } else {
            ALGORAND_TESTNET_GENESIS_HASH
        }
    }

    /// Returns the genesis ID written into `gen`.
    #[must_use]
    pub fn genesis_id(self) -> &'static str {
        if self == Self::MAINNET {
            ALGORAND_MAINNET_GENESIS_ID
        } else {
            ALGORAND_TESTNET_GENESIS_ID
        }
    }

    /// Returns the default algod endpoint for this network.
    #[must_use]
    pub fn default_algod_url(self) -> &'static str {
        if self == Self::MAINNET {
            ALGORAND_MAINNET_ALGOD_URL
        } else {
            ALGORAND_TESTNET_ALGOD_URL
        }
    }
}

impl Debug for AlgorandChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "AlgorandChainReference({})", self.as_str())
    }
}

impl Display for AlgorandChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AlgorandChainReference {
    type Err = AlgorandChainReferenceFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(parsed) = parse_algorand_reference(s) {
            return Ok(parsed);
        }
        Err(AlgorandChainReferenceFormatError::InvalidReference(
            s.to_owned(),
        ))
    }
}

impl Serialize for AlgorandChainReference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AlgorandChainReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<AlgorandChainReference> for ChainId {
    fn from(value: AlgorandChainReference) -> Self {
        Self::new(ALGORAND_NAMESPACE, value.as_str())
    }
}

impl TryFrom<ChainId> for AlgorandChainReference {
    type Error = AlgorandChainReferenceFormatError;

    fn try_from(value: ChainId) -> Result<Self, Self::Error> {
        let (namespace, reference) = value.into_parts();
        if namespace != ALGORAND_NAMESPACE {
            return Err(AlgorandChainReferenceFormatError::InvalidNamespace(
                namespace,
            ));
        }
        Self::from_str(&reference)
    }
}

/// Error type for parsing Algorand chain references.
#[derive(Debug, thiserror::Error)]
pub enum AlgorandChainReferenceFormatError {
    /// The namespace was not `"algorand"`.
    #[error("Invalid namespace {0}, expected algorand")]
    InvalidNamespace(String),
    /// The reference was not a known Algorand genesis prefix or full hash.
    #[error("Invalid algorand chain reference {0}")]
    InvalidReference(String),
}

/// Returns `true` when `network` is a supported Algorand CAIP-2 identifier.
#[must_use]
pub fn is_algorand_network(network: &str) -> bool {
    normalize_algorand_network(network).is_some()
}

/// Maps a CAIP-2 identifier (canonical or full-hash) to a known network.
#[must_use]
pub fn normalize_algorand_network(network: &str) -> Option<AlgorandChainReference> {
    let reference = network.strip_prefix("algorand:")?;
    parse_algorand_reference(reference)
}

fn parse_algorand_reference(reference: &str) -> Option<AlgorandChainReference> {
    if reference == ALGORAND_MAINNET_CAIP_REF || reference == ALGORAND_MAINNET_GENESIS_HASH_B64 {
        return Some(AlgorandChainReference::MAINNET);
    }
    if reference == ALGORAND_TESTNET_CAIP_REF || reference == ALGORAND_TESTNET_GENESIS_HASH_B64 {
        return Some(AlgorandChainReference::TESTNET);
    }
    None
}

/// An Algorand address: 32-byte public key displayed as 58-char base32.
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct AlgorandAddress([u8; 32]);

impl AlgorandAddress {
    /// The all-zero address (omitted from canonical msgpack).
    pub const ZERO: Self = Self([0u8; 32]);

    /// Creates an address from a 32-byte public key.
    #[must_use]
    pub const fn from_public_key(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the 32-byte public key.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns `true` when this is the zero address.
    #[must_use]
    pub fn is_zero(self) -> bool {
        self.0 == [0u8; 32]
    }
}

impl Debug for AlgorandAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "AlgorandAddress({self})")
    }
}

impl Display for AlgorandAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&encode_address(&self.0))
    }
}

impl FromStr for AlgorandAddress {
    type Err = AlgorandAddressFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        decode_address(s)
    }
}

impl Serialize for AlgorandAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for AlgorandAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Errors that can occur when parsing an Algorand address.
#[derive(Debug, thiserror::Error)]
pub enum AlgorandAddressFormatError {
    /// The string is not a 58-character RFC4648 base32 address.
    #[error("invalid algorand address: {0}")]
    Invalid(String),
}

fn address_checksum(public_key: &[u8; 32]) -> [u8; 4] {
    let digest = Sha512_256::digest(public_key);
    let mut checksum = [0u8; 4];
    checksum.copy_from_slice(digest.get(28..32).unwrap_or(&[0; 4]));
    checksum
}

fn encode_address(public_key: &[u8; 32]) -> String {
    let mut payload = [0u8; 36];
    let (pk_slot, checksum_slot) = payload.split_at_mut(32);
    pk_slot.copy_from_slice(public_key);
    checksum_slot.copy_from_slice(&address_checksum(public_key));
    BASE32_NOPAD.encode(&payload)
}

fn decode_address(s: &str) -> Result<AlgorandAddress, AlgorandAddressFormatError> {
    if s.len() != 58 {
        return Err(AlgorandAddressFormatError::Invalid(s.to_owned()));
    }
    let upper = s.to_ascii_uppercase();
    let decoded = BASE32_NOPAD
        .decode(upper.as_bytes())
        .map_err(|_| AlgorandAddressFormatError::Invalid(s.to_owned()))?;
    if decoded.len() != 36 {
        return Err(AlgorandAddressFormatError::Invalid(s.to_owned()));
    }
    let mut public_key = [0u8; 32];
    let Some(pk) = decoded.get(..32) else {
        return Err(AlgorandAddressFormatError::Invalid(s.to_owned()));
    };
    public_key.copy_from_slice(pk);
    let expected = address_checksum(&public_key);
    let got = decoded.get(32..36).unwrap_or(&[]);
    if got != expected {
        return Err(AlgorandAddressFormatError::Invalid(s.to_owned()));
    }
    Ok(AlgorandAddress(public_key))
}

/// Information about an ASA deployment on an Algorand network.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct AlgorandTokenDeployment {
    /// The Algorand network where this token is deployed.
    pub chain_reference: AlgorandChainReference,
    /// The ASA identifier.
    pub asset_id: u64,
    /// The number of decimal places for this token.
    pub decimals: u8,
}

impl AlgorandTokenDeployment {
    /// Creates a new token deployment.
    #[must_use]
    pub const fn new(chain_reference: AlgorandChainReference, asset_id: u64, decimals: u8) -> Self {
        Self {
            chain_reference,
            asset_id,
            decimals,
        }
    }

    /// Creates a deployed token amount with the given raw ASA units.
    #[must_use]
    pub const fn amount(&self, v: u64) -> DeployedTokenAmount<u64, Self> {
        DeployedTokenAmount {
            amount: v,
            token: *self,
        }
    }

    /// Parses a human-readable amount into a deployed token amount.
    ///
    /// # Errors
    ///
    /// Returns [`MoneyAmountParseError`] if the value cannot be parsed, exceeds
    /// precision, or overflows `u64`.
    pub fn parse<V>(&self, v: V) -> Result<DeployedTokenAmount<u64, Self>, MoneyAmountParseError>
    where
        V: TryInto<MoneyAmount>,
        MoneyAmountParseError: From<<V as TryInto<MoneyAmount>>::Error>,
    {
        let amount = v.try_into()?.to_token_amount(self.decimals)?;
        Ok(DeployedTokenAmount {
            amount,
            token: *self,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn address_roundtrip() {
        let pk = [7u8; 32];
        let addr = AlgorandAddress::from_public_key(pk);
        let encoded = addr.to_string();
        assert_eq!(encoded.len(), 58, "algorand addresses are 58 chars");
        let back: AlgorandAddress = encoded.parse().unwrap();
        assert_eq!(back, addr);
        assert!(encoded.parse::<AlgorandAddress>().is_ok());
        assert!("short".parse::<AlgorandAddress>().is_err());
        let mut bad = encoded;
        // Flip a character that is valid base32 so decode succeeds but checksum fails.
        let flipped = if bad.ends_with('A') { "B" } else { "A" };
        bad.replace_range(57.., flipped);
        assert!(bad.parse::<AlgorandAddress>().is_err());
    }

    #[test]
    fn chain_reference_roundtrip() {
        let chain_id: ChainId = AlgorandChainReference::TESTNET.into();
        assert_eq!(
            chain_id.to_string(),
            "algorand:SGO1GKSzyE7IEPItTxCByw9x8FmnrCDe"
        );
        let back = AlgorandChainReference::try_from(chain_id).unwrap();
        assert_eq!(back, AlgorandChainReference::TESTNET);
        assert_eq!(back.genesis_hash(), ALGORAND_TESTNET_GENESIS_HASH);
        assert_eq!(back.genesis_hash_b64, ALGORAND_TESTNET_GENESIS_HASH_B64);
        assert!(is_algorand_network(
            "algorand:wGHE2Pwdvd7S12BL5FaOP20EGYesN73k"
        ));
        assert_eq!(
            normalize_algorand_network("algorand:wGHE2Pwdvd7S12BL5FaOP20EGYesN73ktiC1qzkkit8="),
            Some(AlgorandChainReference::MAINNET)
        );
        assert!(!is_algorand_network("eip155:1"));
    }

    #[test]
    fn token_deployment_parse() {
        let deployment =
            AlgorandTokenDeployment::new(AlgorandChainReference::TESTNET, 10_458_941, 6);
        let parsed = deployment.parse("10.50").unwrap();
        assert_eq!(parsed.amount, 10_500_000);
    }

    #[test]
    fn mainnet_gh_is_full_hash_not_caip_truncation() {
        assert_ne!(
            AlgorandChainReference::MAINNET.as_str(),
            AlgorandChainReference::MAINNET.genesis_hash_b64
        );
        assert!(
            AlgorandChainReference::MAINNET
                .genesis_hash_b64
                .ends_with('=')
        );
    }
}
