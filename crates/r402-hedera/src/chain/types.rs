//! Wire format types for Hedera chain interactions.

use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;

use compact_str::CompactString;
use r402_core::amount::{MoneyAmount, MoneyAmountParseError};
use r402_core::chain::{ChainId, DeployedTokenAmount};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The CAIP-2 namespace for Hedera chains.
pub const HEDERA_NAMESPACE: &str = "hedera";

/// Native HBAR asset id used on the x402 wire.
pub const HBAR_ASSET_ID: &str = "0.0.0";

/// Tinybar decimals for native HBAR.
pub const HBAR_DECIMALS: u8 = 8;

/// USDC decimals on Hedera HTS.
pub const USDC_DECIMALS: u8 = 6;

/// Circle USDC token id on Hedera mainnet.
pub const HEDERA_MAINNET_USDC: &str = "0.0.456858";

/// USDC token id on Hedera testnet.
pub const HEDERA_TESTNET_USDC: &str = "0.0.429274";

/// Public Mirror Node REST base URL for mainnet.
pub const HEDERA_MAINNET_MIRROR_NODE_URL: &str = "https://mainnet-public.mirrornode.hedera.com";

/// Public Mirror Node REST base URL for testnet.
pub const HEDERA_TESTNET_MIRROR_NODE_URL: &str = "https://testnet.mirrornode.hedera.com";

/// A Hedera chain reference (`mainnet` or `testnet`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum HederaChainReference {
    /// Hedera mainnet (`hedera:mainnet`).
    Mainnet,
    /// Hedera testnet (`hedera:testnet`).
    Testnet,
}

impl HederaChainReference {
    /// Hedera mainnet (`hedera:mainnet`).
    pub const MAINNET: Self = Self::Mainnet;

    /// Hedera testnet (`hedera:testnet`).
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

    /// Returns the default Mirror Node REST base URL for this network.
    #[must_use]
    pub const fn default_mirror_url(self) -> &'static str {
        match self {
            Self::Mainnet => HEDERA_MAINNET_MIRROR_NODE_URL,
            Self::Testnet => HEDERA_TESTNET_MIRROR_NODE_URL,
        }
    }
}

impl Debug for HederaChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "HederaChainReference({})", self.as_str())
    }
}

impl Display for HederaChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for HederaChainReference {
    type Err = HederaChainReferenceFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mainnet" => Ok(Self::Mainnet),
            "testnet" => Ok(Self::Testnet),
            other => Err(HederaChainReferenceFormatError::InvalidReference(
                other.to_owned(),
            )),
        }
    }
}

impl Serialize for HederaChainReference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for HederaChainReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<HederaChainReference> for ChainId {
    fn from(value: HederaChainReference) -> Self {
        Self::new(HEDERA_NAMESPACE, value.as_str())
    }
}

impl TryFrom<ChainId> for HederaChainReference {
    type Error = HederaChainReferenceFormatError;

    fn try_from(value: ChainId) -> Result<Self, Self::Error> {
        let (namespace, reference) = value.into_parts();
        if namespace != HEDERA_NAMESPACE {
            return Err(HederaChainReferenceFormatError::InvalidNamespace(namespace));
        }
        Self::from_str(&reference)
            .map_err(|_| HederaChainReferenceFormatError::InvalidReference(reference))
    }
}

/// Error type for parsing Hedera chain references.
#[derive(Debug, thiserror::Error)]
pub enum HederaChainReferenceFormatError {
    /// The namespace was not `"hedera"`.
    #[error("Invalid namespace {0}, expected hedera")]
    InvalidNamespace(String),
    /// The reference was not `mainnet` or `testnet`.
    #[error("Invalid hedera chain reference {0}")]
    InvalidReference(String),
}

/// Returns `true` when `network` is a canonical Hedera CAIP-2 identifier.
#[must_use]
pub fn is_hedera_network(network: &str) -> bool {
    network == "hedera:mainnet" || network == "hedera:testnet"
}

/// Returns `true` when `s` is a Hedera entity id (`shard.realm.num`).
#[must_use]
pub fn is_entity_id(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut dots = 0u8;
    let mut last_dot = true;
    for b in s.bytes() {
        if b == b'.' {
            if last_dot || dots == 2 {
                return false;
            }
            dots = dots.saturating_add(1);
            last_dot = true;
            continue;
        }
        if !b.is_ascii_digit() {
            return false;
        }
        last_dot = false;
    }
    dots == 2 && !last_dot
}

/// Returns `true` when `asset` is native HBAR.
#[must_use]
pub fn is_hbar_asset(asset: &str) -> bool {
    asset == HBAR_ASSET_ID
}

/// Returns `true` when `asset` is HBAR or an HTS token entity id.
#[must_use]
pub fn is_valid_asset(asset: &str) -> bool {
    is_hbar_asset(asset) || is_entity_id(asset)
}

/// A Hedera account, token, or alias identifier.
///
/// Entity ids (`0.0.1234`) are always accepted. When the `client` or
/// `facilitator` feature is on, Hiero account aliases (EVM address /
/// public-key) are also accepted so `aliasPolicy = allow` destinations
/// round-trip.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct HederaAddress(CompactString);

impl HederaAddress {
    /// Returns the identifier as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Returns `true` when this identifier is `shard.realm.num`.
    #[must_use]
    pub fn is_entity_id(&self) -> bool {
        is_entity_id(self.as_str())
    }
}

impl Display for HederaAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for HederaAddress {
    type Err = HederaAddressFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || s.chars().all(char::is_whitespace) {
            return Err(HederaAddressFormatError::Invalid(s.to_owned()));
        }
        if is_entity_id(s) {
            return Ok(Self(CompactString::from(s)));
        }
        #[cfg(any(feature = "client", feature = "facilitator"))]
        {
            if hedera::AccountId::from_str(s).is_ok() {
                return Ok(Self(CompactString::from(s)));
            }
        }
        Err(HederaAddressFormatError::Invalid(s.to_owned()))
    }
}

impl Serialize for HederaAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for HederaAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl AsRef<str> for HederaAddress {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// Errors that can occur when parsing a Hedera identifier.
#[derive(Debug, thiserror::Error)]
pub enum HederaAddressFormatError {
    /// The string is not a Hedera entity id or alias.
    #[error("invalid hedera address: {0}")]
    Invalid(String),
}

/// Returns `true` when two identifiers refer to the same Hedera account.
#[must_use]
pub fn hedera_account_ids_equal(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    #[cfg(any(feature = "client", feature = "facilitator"))]
    {
        if let (Ok(l), Ok(r)) = (
            hedera::AccountId::from_str(left),
            hedera::AccountId::from_str(right),
        ) {
            return l == r;
        }
    }
    false
}

/// HBAR tinybars or HTS atomic units as a decimal string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HederaTokenAmount(CompactString);

impl HederaTokenAmount {
    /// Returns the decimal string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Parses the amount as `u64`.
    ///
    /// # Errors
    ///
    /// Returns [`HederaTokenAmountFormatError`] if the string is not a decimal `u64`.
    pub fn as_u64(&self) -> Result<u64, HederaTokenAmountFormatError> {
        self.0
            .parse()
            .map_err(|_| HederaTokenAmountFormatError::Invalid(self.0.to_string()))
    }
}

impl Display for HederaTokenAmount {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for HederaTokenAmount {
    type Err = HederaTokenAmountFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(HederaTokenAmountFormatError::Invalid(s.to_owned()));
        }
        let _: u64 = s
            .parse()
            .map_err(|_| HederaTokenAmountFormatError::Invalid(s.to_owned()))?;
        Ok(Self(CompactString::from(s)))
    }
}

impl Serialize for HederaTokenAmount {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for HederaTokenAmount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<u64> for HederaTokenAmount {
    fn from(value: u64) -> Self {
        Self(CompactString::from(value.to_string()))
    }
}

impl TryFrom<HederaTokenAmount> for u64 {
    type Error = HederaTokenAmountFormatError;

    fn try_from(value: HederaTokenAmount) -> Result<Self, Self::Error> {
        value.as_u64()
    }
}

/// Error parsing a Hedera token amount.
#[derive(Debug, thiserror::Error)]
pub enum HederaTokenAmountFormatError {
    /// The string is not an unsigned decimal integer.
    #[error("invalid hedera token amount: {0}")]
    Invalid(String),
}

/// Information about an HBAR or HTS token deployment on a Hedera network.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HederaTokenDeployment {
    /// The Hedera network where this token is deployed.
    pub chain_reference: HederaChainReference,
    /// HBAR (`0.0.0`) or an HTS token entity id.
    pub address: HederaAddress,
    /// The number of decimal places for this token.
    pub decimals: u8,
}

impl HederaTokenDeployment {
    /// Creates a new token deployment.
    #[must_use]
    pub const fn new(
        chain_reference: HederaChainReference,
        address: HederaAddress,
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
    fn entity_id_validation() {
        assert!(is_entity_id("0.0.1234"));
        assert!(is_entity_id("0.0.0"));
        assert!(!is_entity_id(""));
        assert!(!is_entity_id("0.0"));
        assert!(!is_entity_id("0.0.1234.5"));
        assert!(!is_entity_id(".0.1"));
        assert!(!is_entity_id("0.0."));
        assert!(!is_entity_id("not-an-account"));
        assert!(!is_entity_id("0x000000000000000000000000000000000000abcd"));
    }

    #[test]
    fn address_entity_and_reject_garbage() {
        assert!("0.0.5001".parse::<HederaAddress>().is_ok());
        assert!("not-an-account".parse::<HederaAddress>().is_err());
        assert!("".parse::<HederaAddress>().is_err());
    }

    #[test]
    fn token_amount_decimal_string() {
        let amount: HederaTokenAmount = "1000".parse().unwrap();
        assert_eq!(amount.as_u64().unwrap(), 1000);
        assert!("".parse::<HederaTokenAmount>().is_err());
        assert!("1.0".parse::<HederaTokenAmount>().is_err());
        assert!("-1".parse::<HederaTokenAmount>().is_err());
    }

    #[test]
    fn chain_reference_roundtrip() {
        let chain_id: ChainId = HederaChainReference::TESTNET.into();
        assert_eq!(chain_id.to_string(), "hedera:testnet");
        let back = HederaChainReference::try_from(chain_id).unwrap();
        assert_eq!(back, HederaChainReference::TESTNET);
        assert!(is_hedera_network("hedera:mainnet"));
        assert!(!is_hedera_network("eip155:1"));
    }

    #[test]
    fn token_deployment_parse() {
        let addr: HederaAddress = HEDERA_TESTNET_USDC.parse().unwrap();
        let deployment = HederaTokenDeployment::new(HederaChainReference::TESTNET, addr, 6);
        let parsed = deployment.parse("10.50").unwrap();
        assert_eq!(parsed.amount, 10_500_000);
    }

    #[test]
    fn asset_helpers() {
        assert!(is_hbar_asset(HBAR_ASSET_ID));
        assert!(is_valid_asset(HBAR_ASSET_ID));
        assert!(is_valid_asset(HEDERA_MAINNET_USDC));
        assert!(!is_valid_asset("invalid-asset"));
    }
}
