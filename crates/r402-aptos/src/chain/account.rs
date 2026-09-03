//! Aptos addresses, chain references, and fungible-asset deployments.

use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;

use compact_str::CompactString;
use r402_protocol::money::{MoneyAmount, MoneyAmountParseError};
use r402_protocol::network::{ChainId, DeployedTokenAmount};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Default USDC decimal precision.
pub const DEFAULT_TOKEN_DECIMALS: u8 = 6;

/// Sponsored-transaction cap on `max_gas_amount` (`@x402/aptos` `MAX_GAS_AMOUNT`).
pub const MAX_GAS_AMOUNT: u64 = 500_000;

/// Sponsored-transaction cap on `gas_unit_price` octas (`MAX_GAS_UNIT_PRICE`).
pub const MAX_GAS_UNIT_PRICE: u64 = 1_000;

/// Client `max_gas_amount` — Aptos SDK simple-tx default, under [`MAX_GAS_AMOUNT`].
pub const DEFAULT_CLIENT_MAX_GAS: u64 = 200_000;

/// Client `gas_unit_price` when estimate is unavailable.
pub const DEFAULT_GAS_UNIT_PRICE: u64 = 100;

/// Verify rejects expirations closer than this many seconds.
pub const EXPIRATION_BUFFER_SECONDS: u64 = 5;

/// `0x1::primary_fungible_store::transfer` (client-built path).
pub const PRIMARY_FUNGIBLE_STORE_TRANSFER: &str = "0x1::primary_fungible_store::transfer";

/// `0x1::fungible_asset::transfer` (also accepted at verify).
pub const FUNGIBLE_ASSET_TRANSFER: &str = "0x1::fungible_asset::transfer";

/// Type argument on both transfer entry functions.
pub const FUNGIBLE_ASSET_METADATA: &str = "0x1::fungible_asset::Metadata";

/// The CAIP-2 namespace for Aptos chains.
pub const APTOS_NAMESPACE: &str = "aptos";

/// Aptos mainnet fullnode REST base.
pub const APTOS_MAINNET_FULLNODE_URL: &str = "https://fullnode.mainnet.aptoslabs.com/v1";

/// Aptos testnet fullnode REST base.
pub const APTOS_TESTNET_FULLNODE_URL: &str = "https://fullnode.testnet.aptoslabs.com/v1";

/// Circle USDC FA metadata on Aptos mainnet.
pub const USDC_MAINNET_FA: &str =
    "0xbae207659db88bea0cbead6da0ed00aac12edcdda169e591cd41c94180b46f3b";

/// Circle USDC FA metadata on Aptos testnet.
pub const USDC_TESTNET_FA: &str =
    "0x69091fbab5f7d635ee7ac5098cf0c1efbe31d68fec0f2cd565e8d168daf52832";

/// A 32-byte Aptos address: `0x` plus exactly 64 hex characters.
const ADDRESS_LEN: usize = 66;

/// An Aptos chain reference (`1` mainnet or `2` testnet).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum AptosChainReference {
    /// Aptos mainnet (`aptos:1`).
    Mainnet,
    /// Aptos testnet (`aptos:2`).
    Testnet,
}

impl AptosChainReference {
    /// Aptos mainnet (`aptos:1`).
    pub const MAINNET: Self = Self::Mainnet;

    /// Aptos testnet (`aptos:2`).
    pub const TESTNET: Self = Self::Testnet;

    /// All chain references with built-in support.
    pub const ALL: &'static [Self] = &[Self::Mainnet, Self::Testnet];

    /// Returns the CAIP-2 reference string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mainnet => "1",
            Self::Testnet => "2",
        }
    }

    /// Aptos on-chain numeric chain id.
    #[must_use]
    pub const fn chain_id(self) -> u8 {
        match self {
            Self::Mainnet => 1,
            Self::Testnet => 2,
        }
    }

    /// Default fullnode REST URL for this network.
    #[must_use]
    pub const fn default_fullnode_url(self) -> &'static str {
        match self {
            Self::Mainnet => APTOS_MAINNET_FULLNODE_URL,
            Self::Testnet => APTOS_TESTNET_FULLNODE_URL,
        }
    }
}

impl Debug for AptosChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "AptosChainReference({})", self.as_str())
    }
}

impl Display for AptosChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AptosChainReference {
    type Err = AptosChainReferenceFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "1" => Ok(Self::Mainnet),
            "2" => Ok(Self::Testnet),
            other => Err(AptosChainReferenceFormatError::InvalidReference(
                other.to_owned(),
            )),
        }
    }
}

impl Serialize for AptosChainReference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AptosChainReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<AptosChainReference> for ChainId {
    fn from(value: AptosChainReference) -> Self {
        Self::new(APTOS_NAMESPACE, value.as_str())
    }
}

impl TryFrom<ChainId> for AptosChainReference {
    type Error = AptosChainReferenceFormatError;

    fn try_from(value: ChainId) -> Result<Self, Self::Error> {
        let (namespace, reference) = value.into_parts();
        if namespace != APTOS_NAMESPACE {
            return Err(AptosChainReferenceFormatError::InvalidNamespace(namespace));
        }
        Self::from_str(&reference)
            .map_err(|_| AptosChainReferenceFormatError::InvalidReference(reference))
    }
}

/// Error type for parsing Aptos chain references.
#[derive(Debug, thiserror::Error)]
pub enum AptosChainReferenceFormatError {
    /// The namespace was not `"aptos"`.
    #[error("Invalid namespace {0}, expected aptos")]
    InvalidNamespace(String),
    /// The reference was not `1` or `2`.
    #[error("Invalid aptos chain reference {0}")]
    InvalidReference(String),
}

/// Returns `true` when `network` is a canonical Aptos CAIP-2 identifier.
#[must_use]
pub fn is_aptos_network(network: &str) -> bool {
    network == "aptos:1" || network == "aptos:2"
}

/// Returns `true` when `s` is `0x` plus exactly 64 hex characters.
#[must_use]
pub fn is_aptos_address(s: &str) -> bool {
    if s.len() != ADDRESS_LEN || !s.starts_with("0x") {
        return false;
    }
    s.as_bytes()
        .get(2..)
        .is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit))
}

/// A 32-byte Aptos account or FA metadata address (`0x` + 64 hex).
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct AptosAddress(CompactString);

impl AptosAddress {
    /// Returns the address as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Display for AptosAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AptosAddress {
    type Err = AptosAddressFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !is_aptos_address(s) {
            return Err(AptosAddressFormatError::Invalid(s.to_owned()));
        }
        Ok(Self(CompactString::from(s)))
    }
}

impl Serialize for AptosAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AptosAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl AsRef<str> for AptosAddress {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// Errors that can occur when parsing an Aptos address.
#[derive(Debug, thiserror::Error)]
pub enum AptosAddressFormatError {
    /// The string is not `0x` + 64 hex characters.
    #[error("invalid aptos address: {0}")]
    Invalid(String),
}

/// Fungible-asset atomic units as a decimal string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AptosTokenAmount(CompactString);

impl AptosTokenAmount {
    /// Returns the decimal string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Parses the amount as `u64`.
    ///
    /// # Errors
    ///
    /// Returns [`AptosTokenAmountFormatError`] if the string is not a decimal `u64`.
    pub fn as_u64(&self) -> Result<u64, AptosTokenAmountFormatError> {
        self.0
            .parse()
            .map_err(|_| AptosTokenAmountFormatError::Invalid(self.0.to_string()))
    }
}

impl Display for AptosTokenAmount {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AptosTokenAmount {
    type Err = AptosTokenAmountFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(AptosTokenAmountFormatError::Invalid(s.to_owned()));
        }
        let _: u64 = s
            .parse()
            .map_err(|_| AptosTokenAmountFormatError::Invalid(s.to_owned()))?;
        Ok(Self(CompactString::from(s)))
    }
}

impl Serialize for AptosTokenAmount {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AptosTokenAmount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<u64> for AptosTokenAmount {
    fn from(value: u64) -> Self {
        Self(CompactString::from(value.to_string()))
    }
}

impl TryFrom<AptosTokenAmount> for u64 {
    type Error = AptosTokenAmountFormatError;

    fn try_from(value: AptosTokenAmount) -> Result<Self, Self::Error> {
        value.as_u64()
    }
}

/// Error parsing an Aptos token amount.
#[derive(Debug, thiserror::Error)]
pub enum AptosTokenAmountFormatError {
    /// The string is not an unsigned decimal integer.
    #[error("invalid aptos token amount: {0}")]
    Invalid(String),
}

/// Information about a fungible-asset deployment on an Aptos network.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AptosTokenDeployment {
    /// The Aptos network where this asset is deployed.
    pub chain_reference: AptosChainReference,
    /// FA metadata object address.
    pub address: AptosAddress,
    /// The number of decimal places for this token.
    pub decimals: u8,
}

impl AptosTokenDeployment {
    /// Creates a new token deployment.
    #[must_use]
    pub const fn new(
        chain_reference: AptosChainReference,
        address: AptosAddress,
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
    pub fn amount(&self, v: u64) -> DeployedTokenAmount<u64, Self> {
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
    /// precision, or overflows `u64`.
    pub fn parse<V>(&self, v: V) -> Result<DeployedTokenAmount<u64, Self>, MoneyAmountParseError>
    where
        V: TryInto<MoneyAmount>,
        MoneyAmountParseError: From<<V as TryInto<MoneyAmount>>::Error>,
    {
        let amount: u64 = v.try_into()?.to_token_amount(self.decimals)?;
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
    fn address_requires_long_hex() {
        assert!(USDC_TESTNET_FA.parse::<AptosAddress>().is_ok());
        assert!("0x1".parse::<AptosAddress>().is_err());
        assert!("".parse::<AptosAddress>().is_err());
        assert!(
            "0x0000000000000000000000000000000000000000000000000000000000000001"
                .parse::<AptosAddress>()
                .is_ok()
        );
        assert!(!is_aptos_address(
            "0xG000000000000000000000000000000000000000000000000000000000000001"
        ));
    }

    #[test]
    fn token_amount_decimal_string() {
        let amount: AptosTokenAmount = "1000".parse().unwrap();
        assert_eq!(amount.as_u64().unwrap(), 1000);
        assert!("".parse::<AptosTokenAmount>().is_err());
        assert!("1.0".parse::<AptosTokenAmount>().is_err());
        assert!("-1".parse::<AptosTokenAmount>().is_err());
    }

    #[test]
    fn chain_reference_roundtrip() {
        let chain_id: ChainId = AptosChainReference::TESTNET.into();
        assert_eq!(chain_id.to_string(), "aptos:2");
        let back = AptosChainReference::try_from(chain_id).unwrap();
        assert_eq!(back, AptosChainReference::TESTNET);
        assert_eq!(AptosChainReference::MAINNET.chain_id(), 1);
        assert!(is_aptos_network("aptos:1"));
        assert!(!is_aptos_network("eip155:1"));
    }

    #[test]
    fn token_deployment_parse() {
        let addr: AptosAddress = USDC_TESTNET_FA.parse().unwrap();
        let deployment = AptosTokenDeployment::new(AptosChainReference::TESTNET, addr, 6);
        let parsed = deployment.parse("10.50").unwrap();
        assert_eq!(parsed.amount, 10_500_000);
    }
}
