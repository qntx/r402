//! Wire format types for TON chain interactions.

use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;

use compact_str::CompactString;
use r402_core::amount::{MoneyAmount, MoneyAmountParseError};
use r402_core::chain::{ChainId, DeployedTokenAmount};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tonlib_core::TonAddress;

use crate::{TONCENTER_MAINNET_BASE_URL, TONCENTER_TESTNET_BASE_URL};

/// The CAIP-2 namespace for TON chains.
pub const TVM_NAMESPACE: &str = "tvm";

/// A TON chain reference (`-239` mainnet or `-3` testnet).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum TvmChainReference {
    /// TON mainnet (`tvm:-239`).
    Mainnet,
    /// TON testnet (`tvm:-3`).
    Testnet,
}

impl TvmChainReference {
    /// TON mainnet (`tvm:-239`).
    pub const MAINNET: Self = Self::Mainnet;

    /// TON testnet (`tvm:-3`).
    pub const TESTNET: Self = Self::Testnet;

    /// All chain references with built-in support.
    pub const ALL: &'static [Self] = &[Self::Mainnet, Self::Testnet];

    /// Returns the CAIP-2 reference string, including the leading minus.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mainnet => "-239",
            Self::Testnet => "-3",
        }
    }

    /// TON global id used when deriving W5R1 `walletId`.
    #[must_use]
    pub const fn global_id(self) -> i32 {
        match self {
            Self::Mainnet => -239,
            Self::Testnet => -3,
        }
    }

    /// Returns the default Toncenter REST root for this network.
    #[must_use]
    pub const fn default_rpc_url(self) -> &'static str {
        match self {
            Self::Mainnet => TONCENTER_MAINNET_BASE_URL,
            Self::Testnet => TONCENTER_TESTNET_BASE_URL,
        }
    }
}

impl Debug for TvmChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "TvmChainReference({})", self.as_str())
    }
}

impl Display for TvmChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TvmChainReference {
    type Err = TvmChainReferenceFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "-239" => Ok(Self::Mainnet),
            "-3" => Ok(Self::Testnet),
            other => Err(TvmChainReferenceFormatError::InvalidReference(
                other.to_owned(),
            )),
        }
    }
}

impl Serialize for TvmChainReference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TvmChainReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<TvmChainReference> for ChainId {
    fn from(value: TvmChainReference) -> Self {
        Self::new(TVM_NAMESPACE, value.as_str())
    }
}

impl TryFrom<ChainId> for TvmChainReference {
    type Error = TvmChainReferenceFormatError;

    fn try_from(value: ChainId) -> Result<Self, Self::Error> {
        let (namespace, reference) = value.into_parts();
        if namespace != TVM_NAMESPACE {
            return Err(TvmChainReferenceFormatError::InvalidNamespace(namespace));
        }
        Self::from_str(&reference)
            .map_err(|_| TvmChainReferenceFormatError::InvalidReference(reference))
    }
}

/// Error type for parsing TON chain references.
#[derive(Debug, thiserror::Error)]
pub enum TvmChainReferenceFormatError {
    /// The namespace was not `"tvm"`.
    #[error("Invalid namespace {0}, expected tvm")]
    InvalidNamespace(String),
    /// The reference was not `-239` or `-3`.
    #[error("Invalid tvm chain reference {0}")]
    InvalidReference(String),
}

/// Returns `true` when `network` is a canonical TVM CAIP-2 identifier.
#[must_use]
pub fn is_tvm_network(network: &str) -> bool {
    network == "tvm:-239" || network == "tvm:-3"
}

/// A TON address stored in raw `workchain:hex` form.
///
/// User-friendly bounceable / non-bounceable strings are accepted on parse
/// and normalized to raw form.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct TvmAddress(CompactString);

impl TvmAddress {
    /// Returns the raw `workchain:hex` string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Parses into a `tonlib-core` address.
    ///
    /// # Errors
    ///
    /// Returns [`TvmAddressFormatError`] if the stored raw form is not a TON address.
    pub fn to_ton(&self) -> Result<TonAddress, TvmAddressFormatError> {
        TonAddress::from_str(self.as_str())
            .map_err(|e| TvmAddressFormatError::Invalid(e.to_string()))
    }
}

impl Display for TvmAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TvmAddress {
    type Err = TvmAddressFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parsed =
            TonAddress::from_str(s).map_err(|e| TvmAddressFormatError::Invalid(e.to_string()))?;
        Ok(Self(CompactString::from(parsed.to_hex())))
    }
}

impl TryFrom<&TonAddress> for TvmAddress {
    type Error = TvmAddressFormatError;

    fn try_from(value: &TonAddress) -> Result<Self, Self::Error> {
        Ok(Self(CompactString::from(value.to_hex())))
    }
}

impl Serialize for TvmAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TvmAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl AsRef<str> for TvmAddress {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// Errors that can occur when parsing a TON address.
#[derive(Debug, thiserror::Error)]
pub enum TvmAddressFormatError {
    /// The string is not a valid TON address.
    #[error("invalid tvm address: {0}")]
    Invalid(String),
}

/// Jetton atomic units as a decimal string; parsed as `u128`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TvmTokenAmount(CompactString);

impl TvmTokenAmount {
    /// Returns the decimal string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Parses the amount as `u128`.
    ///
    /// # Errors
    ///
    /// Returns [`TvmTokenAmountFormatError`] if the string is not a decimal `u128`.
    pub fn as_u128(&self) -> Result<u128, TvmTokenAmountFormatError> {
        self.0
            .parse()
            .map_err(|_| TvmTokenAmountFormatError::Invalid(self.0.to_string()))
    }
}

impl Display for TvmTokenAmount {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TvmTokenAmount {
    type Err = TvmTokenAmountFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(TvmTokenAmountFormatError::Invalid(s.to_owned()));
        }
        let _: u128 = s
            .parse()
            .map_err(|_| TvmTokenAmountFormatError::Invalid(s.to_owned()))?;
        Ok(Self(CompactString::from(s)))
    }
}

impl Serialize for TvmTokenAmount {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TvmTokenAmount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<u128> for TvmTokenAmount {
    fn from(value: u128) -> Self {
        Self(CompactString::from(value.to_string()))
    }
}

impl TryFrom<TvmTokenAmount> for u128 {
    type Error = TvmTokenAmountFormatError;

    fn try_from(value: TvmTokenAmount) -> Result<Self, Self::Error> {
        value.as_u128()
    }
}

/// Error parsing a jetton token amount.
#[derive(Debug, thiserror::Error)]
pub enum TvmTokenAmountFormatError {
    /// The string is not an unsigned decimal integer.
    #[error("invalid tvm token amount: {0}")]
    Invalid(String),
}

/// Information about a jetton minter deployment on a TON network.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TvmTokenDeployment {
    /// The TON network where this token is deployed.
    pub chain_reference: TvmChainReference,
    /// The jetton master contract address.
    pub address: TvmAddress,
    /// The number of decimal places for this token.
    pub decimals: u8,
}

impl TvmTokenDeployment {
    /// Creates a new token deployment.
    #[must_use]
    pub const fn new(
        chain_reference: TvmChainReference,
        address: TvmAddress,
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
    fn chain_id_roundtrips_signed_reference() {
        let mainnet: ChainId = TvmChainReference::MAINNET.into();
        assert_eq!(mainnet.to_string(), "tvm:-239");
        assert_eq!(mainnet.namespace(), "tvm");
        assert_eq!(mainnet.reference(), "-239");
        let back = TvmChainReference::try_from(mainnet).unwrap();
        assert_eq!(back, TvmChainReference::MAINNET);

        let testnet: ChainId = TvmChainReference::TESTNET.into();
        assert_eq!(testnet.to_string(), "tvm:-3");
        assert_eq!(testnet.reference(), "-3");
        assert!(is_tvm_network("tvm:-239"));
        assert!(is_tvm_network("tvm:-3"));
        assert!(!is_tvm_network("eip155:1"));
    }

    #[test]
    fn address_normalizes_raw_and_friendly() {
        let raw = "0:b113a994b5024a16719f69139328eb759596c38a25f59028b146fecdc3621dfe";
        let parsed: TvmAddress = raw.parse().unwrap();
        assert_eq!(parsed.as_str(), raw);
        let friendly = parsed.to_ton().unwrap().to_base64_url();
        let from_friendly: TvmAddress = friendly.parse().unwrap();
        assert_eq!(from_friendly.as_str(), raw);
    }

    #[test]
    fn token_amount_decimal_string() {
        let amount: TvmTokenAmount = "10000".parse().unwrap();
        assert_eq!(amount.as_u128().unwrap(), 10_000);
        assert!("".parse::<TvmTokenAmount>().is_err());
        assert!("1.0".parse::<TvmTokenAmount>().is_err());
        assert!("-1".parse::<TvmTokenAmount>().is_err());
    }
}
