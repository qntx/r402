//! Wire format types for NEAR chain interactions.

use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;

use compact_str::CompactString;
use r402_core::amount::{MoneyAmount, MoneyAmountParseError};
use r402_core::chain::{ChainId, DeployedTokenAmount};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The CAIP-2 namespace for NEAR chains.
pub const NEAR_NAMESPACE: &str = "near";

/// FastNEAR public RPC for mainnet.
#[allow(clippy::doc_markdown, reason = "FastNEAR is a product name")]
pub const NEAR_MAINNET_RPC_URL: &str = "https://rpc.mainnet.fastnear.com";

/// FastNEAR public RPC for testnet.
#[allow(clippy::doc_markdown, reason = "FastNEAR is a product name")]
pub const NEAR_TESTNET_RPC_URL: &str = "https://rpc.testnet.fastnear.com";

/// A NEAR chain reference (`mainnet` or `testnet`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum NearChainReference {
    /// NEAR mainnet (`near:mainnet`).
    Mainnet,
    /// NEAR testnet (`near:testnet`).
    Testnet,
}

impl NearChainReference {
    /// NEAR mainnet (`near:mainnet`).
    pub const MAINNET: Self = Self::Mainnet;

    /// NEAR testnet (`near:testnet`).
    pub const TESTNET: Self = Self::Testnet;

    /// All chain references with built-in support.
    pub const ALL: &'static [Self] = &[Self::Mainnet, Self::Testnet];

    /// Returns the CAIP-2 reference string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Testnet => "testnet",
        }
    }

    /// Returns the default JSON-RPC endpoint for this network.
    #[must_use]
    pub const fn default_rpc_url(self) -> &'static str {
        match self {
            Self::Mainnet => NEAR_MAINNET_RPC_URL,
            Self::Testnet => NEAR_TESTNET_RPC_URL,
        }
    }
}

impl Debug for NearChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "NearChainReference({})", self.as_str())
    }
}

impl Display for NearChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for NearChainReference {
    type Err = NearChainReferenceFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mainnet" => Ok(Self::Mainnet),
            "testnet" => Ok(Self::Testnet),
            other => Err(NearChainReferenceFormatError::InvalidReference(
                other.to_owned(),
            )),
        }
    }
}

impl Serialize for NearChainReference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for NearChainReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<NearChainReference> for ChainId {
    fn from(value: NearChainReference) -> Self {
        Self::new(NEAR_NAMESPACE, value.as_str())
    }
}

impl TryFrom<ChainId> for NearChainReference {
    type Error = NearChainReferenceFormatError;

    fn try_from(value: ChainId) -> Result<Self, Self::Error> {
        let (namespace, reference) = value.into_parts();
        if namespace != NEAR_NAMESPACE {
            return Err(NearChainReferenceFormatError::InvalidNamespace(namespace));
        }
        Self::from_str(&reference)
            .map_err(|_| NearChainReferenceFormatError::InvalidReference(reference))
    }
}

/// Error type for parsing NEAR chain references.
#[derive(Debug, thiserror::Error)]
pub enum NearChainReferenceFormatError {
    /// The namespace was not `"near"`.
    #[error("Invalid namespace {0}, expected near")]
    InvalidNamespace(String),
    /// The reference was not `mainnet` or `testnet`.
    #[error("Invalid near chain reference {0}")]
    InvalidReference(String),
}

/// Returns `true` when `network` is a canonical NEAR CAIP-2 identifier.
#[must_use]
pub fn is_near_network(network: &str) -> bool {
    network == "near:mainnet" || network == "near:testnet"
}

/// A NEAR account ID: named (`alice.testnet`) or implicit (64 lowercase hex).
///
/// Stored as [`CompactString`]. Validation matches `near-account-id`.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct NearAddress(CompactString);

impl NearAddress {
    /// Returns the account ID as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Display for NearAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for NearAddress {
    type Err = NearAddressFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        validate_account_id(s)?;
        Ok(Self(CompactString::from(s)))
    }
}

impl Serialize for NearAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for NearAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl AsRef<str> for NearAddress {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// Errors that can occur when parsing a NEAR account ID.
#[derive(Debug, thiserror::Error)]
pub enum NearAddressFormatError {
    /// The string is not a valid NEAR account ID.
    #[error("invalid near account id: {0}")]
    Invalid(String),
}

fn validate_account_id(s: &str) -> Result<(), NearAddressFormatError> {
    let len = s.len();
    if !(2..=64).contains(&len) {
        return Err(NearAddressFormatError::Invalid(s.to_owned()));
    }
    if s.bytes()
        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        && len == 64
    {
        return Ok(());
    }
    let bytes = s.as_bytes();
    let mut prev_separator = true;
    for &b in bytes {
        let is_separator = matches!(b, b'.' | b'-' | b'_');
        if is_separator {
            if prev_separator {
                return Err(NearAddressFormatError::Invalid(s.to_owned()));
            }
            prev_separator = true;
            continue;
        }
        if !b.is_ascii_lowercase() && !b.is_ascii_digit() {
            return Err(NearAddressFormatError::Invalid(s.to_owned()));
        }
        prev_separator = false;
    }
    if prev_separator {
        return Err(NearAddressFormatError::Invalid(s.to_owned()));
    }
    Ok(())
}

/// NEP-141 atomic units as a decimal string; parsed as `u128`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NearTokenAmount(CompactString);

impl NearTokenAmount {
    /// Returns the decimal string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Parses the amount as `u128`.
    ///
    /// # Errors
    ///
    /// Returns [`NearTokenAmountFormatError`] if the string is not a decimal `u128`.
    pub fn as_u128(&self) -> Result<u128, NearTokenAmountFormatError> {
        self.0
            .parse()
            .map_err(|_| NearTokenAmountFormatError::Invalid(self.0.to_string()))
    }
}

impl Display for NearTokenAmount {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for NearTokenAmount {
    type Err = NearTokenAmountFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(NearTokenAmountFormatError::Invalid(s.to_owned()));
        }
        let _: u128 = s
            .parse()
            .map_err(|_| NearTokenAmountFormatError::Invalid(s.to_owned()))?;
        Ok(Self(CompactString::from(s)))
    }
}

impl Serialize for NearTokenAmount {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for NearTokenAmount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<u128> for NearTokenAmount {
    fn from(value: u128) -> Self {
        Self(CompactString::from(value.to_string()))
    }
}

impl TryFrom<NearTokenAmount> for u128 {
    type Error = NearTokenAmountFormatError;

    fn try_from(value: NearTokenAmount) -> Result<Self, Self::Error> {
        value.as_u128()
    }
}

/// Error parsing a NEP-141 token amount.
#[derive(Debug, thiserror::Error)]
pub enum NearTokenAmountFormatError {
    /// The string is not an unsigned decimal integer.
    #[error("invalid near token amount: {0}")]
    Invalid(String),
}

/// Information about a NEP-141 token deployment on a NEAR network.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NearTokenDeployment {
    /// The NEAR network where this token is deployed.
    pub chain_reference: NearChainReference,
    /// The NEP-141 contract account ID.
    pub address: NearAddress,
    /// The number of decimal places for this token.
    pub decimals: u8,
}

impl NearTokenDeployment {
    /// Creates a new token deployment.
    #[must_use]
    pub const fn new(
        chain_reference: NearChainReference,
        address: NearAddress,
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
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn named_and_implicit_addresses() {
        assert!("alice.testnet".parse::<NearAddress>().is_ok());
        assert!(
            "17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1"
                .parse::<NearAddress>()
                .is_ok()
        );
        assert!("Alice.testnet".parse::<NearAddress>().is_err());
        assert!("a".parse::<NearAddress>().is_err());
        assert!(".alice".parse::<NearAddress>().is_err());
        assert!("alice.".parse::<NearAddress>().is_err());
        assert!("alice..testnet".parse::<NearAddress>().is_err());
    }

    #[test]
    fn token_amount_decimal_string() {
        let amount: NearTokenAmount = "1000000".parse().unwrap();
        assert_eq!(amount.as_u128().unwrap(), 1_000_000);
        assert!("".parse::<NearTokenAmount>().is_err());
        assert!("1.0".parse::<NearTokenAmount>().is_err());
        assert!("-1".parse::<NearTokenAmount>().is_err());
    }

    #[test]
    fn chain_reference_roundtrip() {
        let chain_id: ChainId = NearChainReference::TESTNET.into();
        assert_eq!(chain_id.to_string(), "near:testnet");
        let back = NearChainReference::try_from(chain_id).unwrap();
        assert_eq!(back, NearChainReference::TESTNET);
        assert!(is_near_network("near:mainnet"));
        assert!(!is_near_network("eip155:1"));
    }

    #[test]
    fn token_deployment_parse() {
        let addr: NearAddress = "usdc.testnet".parse().unwrap();
        let deployment = NearTokenDeployment::new(NearChainReference::TESTNET, addr, 6);
        let parsed = deployment.parse("10.50").unwrap();
        assert_eq!(parsed.amount, 10_500_000);
    }
}
