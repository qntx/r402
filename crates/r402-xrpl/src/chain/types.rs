//! Wire format types for XRPL chain interactions.

use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;

use compact_str::CompactString;
use r402_core::chain::{ChainId, DeployedTokenAmount};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use xrpl::core::addresscodec::is_valid_classic_address;

/// The CAIP-2 namespace for XRPL chains.
pub const XRPL_NAMESPACE: &str = "xrpl";

/// Default JSON-RPC HTTP endpoint for XRPL mainnet.
pub const XRPL_MAINNET_RPC_URL: &str = "https://s1.ripple.com:51234";

/// Default JSON-RPC HTTP endpoint for XRPL testnet.
pub const XRPL_TESTNET_RPC_URL: &str = "https://s.altnet.rippletest.net:51234";

/// Default JSON-RPC HTTP endpoint for XRPL devnet.
pub const XRPL_DEVNET_RPC_URL: &str = "https://s.devnet.rippletest.net:51234";

/// An XRPL numeric `NetworkID` used as the CAIP-2 reference (`xrpl:{id}`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct XrplChainReference(u32);

impl XrplChainReference {
    /// XRPL mainnet (`xrpl:0`).
    pub const MAINNET: Self = Self(0);

    /// XRPL testnet (`xrpl:1`).
    pub const TESTNET: Self = Self(1);

    /// XRPL devnet (`xrpl:2`).
    pub const DEVNET: Self = Self(2);

    /// Built-in networks with default RPC URLs.
    pub const ALL: &'static [Self] = &[Self::MAINNET, Self::TESTNET, Self::DEVNET];

    /// Constructs a chain reference from a numeric XRPL `NetworkID`.
    #[must_use]
    pub const fn new(network_id: u32) -> Self {
        Self(network_id)
    }

    /// Returns the numeric XRPL `NetworkID`.
    #[must_use]
    pub const fn network_id(self) -> u32 {
        self.0
    }

    /// Returns `true` when XRPL protocol rules require omitting `NetworkID`.
    #[must_use]
    pub const fn is_standard_network(self) -> bool {
        self.0 <= 1024
    }

    /// Returns the CAIP-2 reference string.
    #[must_use]
    pub fn as_str(self) -> CompactString {
        CompactString::from(self.0.to_string())
    }

    /// Returns the default JSON-RPC URL for this network, if known.
    #[must_use]
    pub const fn default_rpc_url(self) -> Option<&'static str> {
        match self.0 {
            0 => Some(XRPL_MAINNET_RPC_URL),
            1 => Some(XRPL_TESTNET_RPC_URL),
            2 => Some(XRPL_DEVNET_RPC_URL),
            _ => None,
        }
    }
}

impl Debug for XrplChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "XrplChainReference({})", self.0)
    }
}

impl Display for XrplChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl FromStr for XrplChainReference {
    type Err = XrplChainReferenceFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let id: u32 = s
            .parse()
            .map_err(|_| XrplChainReferenceFormatError::InvalidReference(s.to_owned()))?;
        Ok(Self(id))
    }
}

impl Serialize for XrplChainReference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for XrplChainReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<XrplChainReference> for ChainId {
    fn from(value: XrplChainReference) -> Self {
        Self::new(XRPL_NAMESPACE, value.0.to_string())
    }
}

impl TryFrom<ChainId> for XrplChainReference {
    type Error = XrplChainReferenceFormatError;

    fn try_from(value: ChainId) -> Result<Self, Self::Error> {
        let (namespace, reference) = value.into_parts();
        if namespace != XRPL_NAMESPACE {
            return Err(XrplChainReferenceFormatError::InvalidNamespace(namespace));
        }
        Self::from_str(&reference)
            .map_err(|_| XrplChainReferenceFormatError::InvalidReference(reference))
    }
}

/// Error type for parsing XRPL chain references.
#[derive(Debug, thiserror::Error)]
pub enum XrplChainReferenceFormatError {
    /// The namespace was not `"xrpl"`.
    #[error("Invalid namespace {0}, expected xrpl")]
    InvalidNamespace(String),
    /// The reference was not an unsigned 32-bit decimal `NetworkID`.
    #[error("Invalid xrpl chain reference {0}")]
    InvalidReference(String),
}

/// Returns `true` when `network` is an XRPL CAIP-2 identifier (`xrpl:{u32}`).
#[must_use]
pub fn is_xrpl_network(network: &str) -> bool {
    let Some((namespace, reference)) = network.split_once(':') else {
        return false;
    };
    namespace == XRPL_NAMESPACE && XrplChainReference::from_str(reference).is_ok()
}

/// Parses the numeric XRPL `NetworkID` from a CAIP-2 network string.
///
/// # Errors
///
/// Returns [`XrplChainReferenceFormatError`] when the identifier is not `xrpl:{u32}`.
pub fn parse_xrpl_network_id(network: &str) -> Result<u32, XrplChainReferenceFormatError> {
    let chain_id = ChainId::from_str(network)
        .map_err(|_| XrplChainReferenceFormatError::InvalidReference(network.to_owned()))?;
    Ok(XrplChainReference::try_from(chain_id)?.network_id())
}

/// An XRPL classic address (`r…`). X-addresses are rejected.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct XrplClassicAddress(CompactString);

impl XrplClassicAddress {
    /// Returns the address as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Display for XrplClassicAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for XrplClassicAddress {
    type Err = XrplClassicAddressFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if is_valid_classic_address(s) {
            Ok(Self(CompactString::from(s)))
        } else {
            Err(XrplClassicAddressFormatError::Invalid(s.to_owned()))
        }
    }
}

impl Serialize for XrplClassicAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for XrplClassicAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl AsRef<str> for XrplClassicAddress {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// Errors that can occur when parsing an XRPL classic address.
#[derive(Debug, thiserror::Error)]
pub enum XrplClassicAddressFormatError {
    /// The string is not a valid XRPL classic address.
    #[error("invalid xrpl classic address: {0}")]
    Invalid(String),
}

/// Wire string used for both `payTo` (classic `r-address`) and `asset`
/// (`XRP`, 3-character currency, or 40-hex currency).
///
/// [`r402_core::wire::PaymentRequirements::as_concrete`] uses one address
/// type for both fields.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct XrplAddress(CompactString);

impl XrplAddress {
    /// Returns the value as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Returns `true` when this value is native XRP.
    #[must_use]
    pub fn is_xrp(&self) -> bool {
        self.0 == "XRP"
    }
}

impl Display for XrplAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for XrplAddress {
    type Err = XrplAddressFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if is_valid_classic_address(s) || is_xrpl_asset_id(s) {
            Ok(Self(CompactString::from(s)))
        } else {
            Err(XrplAddressFormatError::Invalid(s.to_owned()))
        }
    }
}

impl Serialize for XrplAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for XrplAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl AsRef<str> for XrplAddress {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<XrplClassicAddress> for XrplAddress {
    fn from(value: XrplClassicAddress) -> Self {
        Self(value.0)
    }
}

/// Errors that can occur when parsing an XRPL pay-to or asset identifier.
#[derive(Debug, thiserror::Error)]
pub enum XrplAddressFormatError {
    /// The string is not a classic address or asset identifier.
    #[error("invalid xrpl address or asset: {0}")]
    Invalid(String),
}

fn is_xrpl_asset_id(s: &str) -> bool {
    if s == "XRP" {
        return true;
    }
    if s.len() == 3 && s.bytes().all(|b| b.is_ascii() && !b.is_ascii_control()) {
        return true;
    }
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Payment amount: integer drops for XRP, or an issued-currency decimal string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XrplTokenAmount(CompactString);

impl XrplTokenAmount {
    /// Returns the amount string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Returns `true` when the amount is an unsigned integer string.
    #[must_use]
    pub fn is_integer(&self) -> bool {
        is_integer_string(self.as_str())
    }
}

impl Display for XrplTokenAmount {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for XrplTokenAmount {
    type Err = XrplTokenAmountFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if is_decimal_string(s) {
            Ok(Self(CompactString::from(s)))
        } else {
            Err(XrplTokenAmountFormatError::Invalid(s.to_owned()))
        }
    }
}

impl Serialize for XrplTokenAmount {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for XrplTokenAmount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<u64> for XrplTokenAmount {
    fn from(value: u64) -> Self {
        Self(CompactString::from(value.to_string()))
    }
}

impl From<u128> for XrplTokenAmount {
    fn from(value: u128) -> Self {
        Self(CompactString::from(value.to_string()))
    }
}

impl From<&str> for XrplTokenAmount {
    fn from(value: &str) -> Self {
        Self(CompactString::from(value))
    }
}

/// Error parsing an XRPL payment amount.
#[derive(Debug, thiserror::Error)]
pub enum XrplTokenAmountFormatError {
    /// The string is not an unsigned integer or decimal.
    #[error("invalid xrpl token amount: {0}")]
    Invalid(String),
}

/// Returns `true` when `value` is an unsigned base-10 integer string.
#[must_use]
pub fn is_integer_string(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

/// Returns `true` when `value` is a non-negative decimal string.
#[must_use]
pub fn is_decimal_string(value: &str) -> bool {
    let mut parts = value.split('.');
    let Some(whole) = parts.next() else {
        return false;
    };
    if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    parts.next().is_none_or(|fraction| {
        !fraction.is_empty()
            && fraction.bytes().all(|b| b.is_ascii_digit())
            && parts.next().is_none()
    })
}

/// An issued currency identified by `(currency, issuer)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XrplIssuedCurrency {
    /// 3-character code or 40-hex currency.
    pub currency: CompactString,
    /// Classic address of the issuer.
    pub issuer: XrplClassicAddress,
}

/// Native XRP or an issued currency.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum XrplAsset {
    /// Native XRP; amount is integer drops.
    Xrp,
    /// Issued currency; amount is an XRPL decimal value string.
    Iou(XrplIssuedCurrency),
}

impl XrplAsset {
    /// Returns the `PaymentRequirements.asset` string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Xrp => "XRP",
            Self::Iou(iou) => iou.currency.as_str(),
        }
    }
}

/// Token deployment metadata for an XRPL asset on one network.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XrplTokenDeployment {
    /// The XRPL network where this asset is used.
    pub chain_reference: XrplChainReference,
    /// Native XRP or an issued currency.
    pub asset: XrplAsset,
    /// Decimal places used when parsing human amounts for this asset.
    pub decimals: u8,
}

impl XrplTokenDeployment {
    /// Creates a new token deployment.
    #[must_use]
    pub const fn new(chain_reference: XrplChainReference, asset: XrplAsset, decimals: u8) -> Self {
        Self {
            chain_reference,
            asset,
            decimals,
        }
    }

    /// Creates a deployed token amount from drops (XRP) or a decimal string (IOU).
    #[must_use]
    pub fn amount(
        &self,
        v: impl Into<XrplTokenAmount>,
    ) -> DeployedTokenAmount<XrplTokenAmount, Self> {
        DeployedTokenAmount {
            amount: v.into(),
            token: self.clone(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn classic_address_rejects_x_address_and_empty() {
        let valid = "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh";
        assert!(valid.parse::<XrplClassicAddress>().is_ok());
        assert!("".parse::<XrplClassicAddress>().is_err());
        assert!("not-an-address".parse::<XrplClassicAddress>().is_err());
        assert!(
            "X7AcgcsBL6XDcUb289X4mJ8djcdyKaB5hJDWMArnXr61cqZ"
                .parse::<XrplClassicAddress>()
                .is_err()
        );
    }

    #[test]
    fn asset_ids_parse_as_address() {
        assert!("XRP".parse::<XrplAddress>().is_ok());
        assert!("USD".parse::<XrplAddress>().is_ok());
        assert!(
            "524C555344000000000000000000000000000000"
                .parse::<XrplAddress>()
                .is_ok()
        );
        assert!("not-an-asset".parse::<XrplAddress>().is_err());
    }

    #[test]
    fn token_amount_decimal_and_integer() {
        let drops: XrplTokenAmount = "1000000".parse().unwrap();
        assert!(drops.is_integer());
        let iou: XrplTokenAmount = "10.5".parse().unwrap();
        assert!(!iou.is_integer());
        assert!("".parse::<XrplTokenAmount>().is_err());
        assert!("10.5.5".parse::<XrplTokenAmount>().is_err());
        assert!("-1".parse::<XrplTokenAmount>().is_err());
    }

    #[test]
    fn chain_reference_roundtrip() {
        let chain_id: ChainId = XrplChainReference::TESTNET.into();
        assert_eq!(chain_id.to_string(), "xrpl:1");
        let back = XrplChainReference::try_from(chain_id).unwrap();
        assert_eq!(back, XrplChainReference::TESTNET);
        assert!(is_xrpl_network("xrpl:0"));
        assert!(is_xrpl_network("xrpl:2"));
        assert!(!is_xrpl_network("eip155:1"));
        assert!(!is_xrpl_network("xrpl"));
        assert!(XrplChainReference::MAINNET.is_standard_network());
        assert!(XrplChainReference::new(1024).is_standard_network());
        assert!(!XrplChainReference::new(1025).is_standard_network());
    }
}
