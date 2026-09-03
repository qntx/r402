//! Keeta addresses, chain references, and token deployments.

use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;
use std::sync::Arc;

use compact_str::CompactString;
use keetanetwork_account::GenericAccount;
use r402_protocol::money::{MoneyAmount, MoneyAmountParseError};
use r402_protocol::network::{ChainId, DeployedTokenAmount};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Default USDC decimal precision.
pub const DEFAULT_TOKEN_DECIMALS: u8 = 6;

/// The CAIP-2 namespace for Keeta chains.
pub const KEETA_NAMESPACE: &str = "keeta";

/// CAIP-2 reference for Keeta mainnet (`Network::Main`, `id() = 0x5382`).
pub const KEETA_MAINNET_REFERENCE: &str = "21378";

/// CAIP-2 reference for Keeta testnet (`Network::Test`, `id() = 0x5445_5354`).
pub const KEETA_TESTNET_REFERENCE: &str = "1413829460";

/// A Keeta chain reference (`21378` or `1413829460`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeetaChainReference {
    /// Keeta mainnet (`keeta:21378`).
    Mainnet,
    /// Keeta testnet (`keeta:1413829460`).
    Testnet,
}

impl KeetaChainReference {
    /// Keeta mainnet (`keeta:21378`).
    pub const MAINNET: Self = Self::Mainnet;

    /// Keeta testnet (`keeta:1413829460`).
    pub const TESTNET: Self = Self::Testnet;

    /// All chain references with built-in support.
    pub const ALL: &'static [Self] = &[Self::Mainnet, Self::Testnet];

    /// Returns the CAIP-2 reference string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mainnet => KEETA_MAINNET_REFERENCE,
            Self::Testnet => KEETA_TESTNET_REFERENCE,
        }
    }

    /// Decimal network id stamped onto blocks (`0x5382` / `0x5445_5354`).
    #[must_use]
    pub const fn network_id(self) -> u32 {
        match self {
            Self::Mainnet => 0x5382,
            Self::Testnet => 0x5445_5354,
        }
    }

    /// Maps this CAIP-2 reference onto `keetanetwork_client::Network`.
    #[cfg(any(feature = "client", feature = "facilitator"))]
    #[must_use]
    pub const fn client_network(self) -> keetanetwork_client::Network {
        match self {
            Self::Mainnet => keetanetwork_client::Network::Main,
            Self::Testnet => keetanetwork_client::Network::Test,
        }
    }
}

impl Debug for KeetaChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "KeetaChainReference({})", self.as_str())
    }
}

impl Display for KeetaChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for KeetaChainReference {
    type Err = KeetaChainReferenceFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            KEETA_MAINNET_REFERENCE => Ok(Self::Mainnet),
            KEETA_TESTNET_REFERENCE => Ok(Self::Testnet),
            other => Err(KeetaChainReferenceFormatError::InvalidReference(
                other.to_owned(),
            )),
        }
    }
}

impl Serialize for KeetaChainReference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for KeetaChainReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<KeetaChainReference> for ChainId {
    fn from(value: KeetaChainReference) -> Self {
        Self::new(KEETA_NAMESPACE, value.as_str())
    }
}

impl TryFrom<ChainId> for KeetaChainReference {
    type Error = KeetaChainReferenceFormatError;

    fn try_from(value: ChainId) -> Result<Self, Self::Error> {
        let (namespace, reference) = value.into_parts();
        if namespace != KEETA_NAMESPACE {
            return Err(KeetaChainReferenceFormatError::InvalidNamespace(namespace));
        }
        Self::from_str(&reference)
            .map_err(|_| KeetaChainReferenceFormatError::InvalidReference(reference))
    }
}

/// Error type for parsing Keeta chain references.
#[derive(Debug, thiserror::Error)]
pub enum KeetaChainReferenceFormatError {
    /// The namespace was not `"keeta"`.
    #[error("Invalid namespace {0}, expected keeta")]
    InvalidNamespace(String),
    /// The reference was not `21378` or `1413829460`.
    #[error("Invalid keeta chain reference {0}")]
    InvalidReference(String),
}

/// Returns `true` when `network` is a canonical Keeta CAIP-2 identifier.
#[must_use]
pub fn is_keeta_network(network: &str) -> bool {
    network == "keeta:21378" || network == "keeta:1413829460"
}

/// A Keeta account or token address (`keeta_…`).
///
/// Stored as [`CompactString`]. Validation uses
/// [`GenericAccount::from_str`].
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct KeetaAddress(CompactString);

impl KeetaAddress {
    /// Returns the address as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Parses into a shared [`GenericAccount`] (public-key material only).
    ///
    /// # Errors
    ///
    /// Returns [`KeetaAddressFormatError`] when the address is not a valid
    /// Keeta public-key string.
    pub fn to_account(&self) -> Result<Arc<GenericAccount>, KeetaAddressFormatError> {
        parse_account(self.as_str())
    }
}

impl Display for KeetaAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for KeetaAddress {
    type Err = KeetaAddressFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let account = parse_account(s)?;
        Ok(Self(CompactString::from(account.to_string())))
    }
}

impl Serialize for KeetaAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for KeetaAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl AsRef<str> for KeetaAddress {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// Errors that can occur when parsing a Keeta address.
#[derive(Debug, thiserror::Error)]
pub enum KeetaAddressFormatError {
    /// The string is not a valid Keeta public-key address.
    #[error("invalid keeta address: {0}")]
    Invalid(String),
}

fn parse_account(s: &str) -> Result<Arc<GenericAccount>, KeetaAddressFormatError> {
    let account =
        GenericAccount::from_str(s).map_err(|e| KeetaAddressFormatError::Invalid(e.to_string()))?;
    Ok(Arc::new(account))
}

/// Returns whether `account` can produce signatures.
#[cfg(any(feature = "client", feature = "facilitator"))]
#[must_use]
pub(crate) fn account_has_private_key(account: &GenericAccount) -> bool {
    match account {
        GenericAccount::Ed25519(inner) => inner.has_private_key(),
        GenericAccount::EcdsaSecp256k1(inner) => inner.has_private_key(),
        GenericAccount::EcdsaSecp256r1(inner) => inner.has_private_key(),
        GenericAccount::Network(_)
        | GenericAccount::Token(_)
        | GenericAccount::Storage(_)
        | GenericAccount::Multisig(_) => false,
    }
}

/// Token atomic units as a decimal string; parsed as `u128`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeetaTokenAmount(CompactString);

impl KeetaTokenAmount {
    /// Returns the decimal string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Parses the amount as `u128`.
    ///
    /// # Errors
    ///
    /// Returns [`KeetaTokenAmountFormatError`] if the string is not a decimal `u128`.
    pub fn as_u128(&self) -> Result<u128, KeetaTokenAmountFormatError> {
        self.0
            .parse()
            .map_err(|_| KeetaTokenAmountFormatError::Invalid(self.0.to_string()))
    }
}

impl Display for KeetaTokenAmount {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for KeetaTokenAmount {
    type Err = KeetaTokenAmountFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(KeetaTokenAmountFormatError::Invalid(s.to_owned()));
        }
        let _: u128 = s
            .parse()
            .map_err(|_| KeetaTokenAmountFormatError::Invalid(s.to_owned()))?;
        Ok(Self(CompactString::from(s)))
    }
}

impl Serialize for KeetaTokenAmount {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for KeetaTokenAmount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<u128> for KeetaTokenAmount {
    fn from(value: u128) -> Self {
        Self(CompactString::from(value.to_string()))
    }
}

impl TryFrom<KeetaTokenAmount> for u128 {
    type Error = KeetaTokenAmountFormatError;

    fn try_from(value: KeetaTokenAmount) -> Result<Self, Self::Error> {
        value.as_u128()
    }
}

/// Error parsing a Keeta token amount.
#[derive(Debug, thiserror::Error)]
pub enum KeetaTokenAmountFormatError {
    /// The string is not an unsigned decimal integer.
    #[error("invalid keeta token amount: {0}")]
    Invalid(String),
}

/// Information about a token deployment on a Keeta network.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeetaTokenDeployment {
    /// The Keeta network where this token is deployed.
    pub chain_reference: KeetaChainReference,
    /// The token account address.
    pub address: KeetaAddress,
    /// The number of decimal places for this token.
    pub decimals: u8,
}

impl KeetaTokenDeployment {
    /// Creates a new token deployment.
    #[must_use]
    pub const fn new(
        chain_reference: KeetaChainReference,
        address: KeetaAddress,
        decimals: u8,
    ) -> Self {
        Self {
            chain_reference,
            address,
            decimals,
        }
    }

    /// Creates a deployed token amount with the given raw atomic units.
    #[must_use]
    pub fn amount(&self, v: u128) -> DeployedTokenAmount<u128, Self> {
        DeployedTokenAmount {
            amount: v,
            token: self.clone(),
        }
    }

    /// Parses a human-readable amount into a deployed token amount.
    ///
    /// # Errors
    ///
    /// Returns [`MoneyAmountParseError`] if the value cannot be parsed, exceeds
    /// precision, or overflows `u128`.
    pub fn parse<V>(&self, v: V) -> Result<DeployedTokenAmount<u128, Self>, MoneyAmountParseError>
    where
        V: TryInto<MoneyAmount>,
        MoneyAmountParseError: From<<V as TryInto<MoneyAmount>>::Error>,
    {
        let amount: u128 = v.try_into()?.to_token_amount(self.decimals)?;
        Ok(DeployedTokenAmount {
            amount,
            token: self.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_usdc_addresses_parse() {
        let mainnet: KeetaAddress =
            "keeta_amnkge74xitii5dsobstldatv3irmyimujfjotftx7plaaaseam4bntb7wnna"
                .parse()
                .unwrap();
        let testnet: KeetaAddress =
            "keeta_apna75yhhvnv4ei7ape55hndk4yepno7a7i2mhtiwahiygixjcnmvswxhnmnk"
                .parse()
                .unwrap();
        assert!(mainnet.as_str().starts_with("keeta_"));
        assert!(testnet.as_str().starts_with("keeta_"));
        assert!("not-an-address".parse::<KeetaAddress>().is_err());
    }

    #[test]
    fn token_amount_decimal_string() {
        let amount: KeetaTokenAmount = "1000000".parse().unwrap();
        assert_eq!(amount.as_u128().unwrap(), 1_000_000);
        assert!("".parse::<KeetaTokenAmount>().is_err());
        assert!("1.0".parse::<KeetaTokenAmount>().is_err());
        assert!("-1".parse::<KeetaTokenAmount>().is_err());
    }

    #[test]
    fn chain_reference_roundtrip() {
        let chain_id: ChainId = KeetaChainReference::TESTNET.into();
        assert_eq!(chain_id.to_string(), "keeta:1413829460");
        let back = KeetaChainReference::try_from(chain_id).unwrap();
        assert_eq!(back, KeetaChainReference::TESTNET);
        assert_eq!(KeetaChainReference::MAINNET.network_id(), 0x5382);
        assert_eq!(KeetaChainReference::TESTNET.network_id(), 0x5445_5354);
        assert!(is_keeta_network("keeta:21378"));
        assert!(!is_keeta_network("eip155:1"));
    }
}
