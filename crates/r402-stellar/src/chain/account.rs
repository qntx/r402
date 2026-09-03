//! Stellar addresses, chain references, and SEP-41 token deployments.

use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;

use compact_str::CompactString;
use r402_protocol::money::{MoneyAmount, MoneyAmountParseError};
use r402_protocol::network::{ChainId, DeployedTokenAmount};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use stellar_strkey::Strkey;

/// The CAIP-2 namespace for Stellar chains.
pub const STELLAR_NAMESPACE: &str = "stellar";

/// Default USDC decimal precision on Stellar (SEP-41).
pub const DEFAULT_TOKEN_DECIMALS: u8 = 7;

/// Inclusion buffer in stroops (`BASE_FEE` in the official Stellar SDK).
pub const BASE_FEE_STROOPS: u32 = 100;

/// Safety ceiling for simulation-derived settlement fees (stroops).
pub const DEFAULT_MAX_TRANSACTION_FEE_STROOPS: u32 = 50_000;

/// Fallback ledger close time when Horizon is unavailable.
pub const DEFAULT_ESTIMATED_LEDGER_SECONDS: u64 = 5;

/// Ledger-skew tolerance applied to auth-entry expiration.
pub const SIGNATURE_EXPIRATION_LEDGER_TOLERANCE: u32 = 2;

/// Default `maxTimeoutSeconds` when the requirement omits it.
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 60;

/// Number of recent Horizon ledgers sampled for close-time estimation.
pub const HORIZON_LEDGERS_SAMPLE_SIZE: u32 = 20;

/// Dummy source used by the client so only auth entries are signed.
///
/// Matches `@stellar/stellar-sdk` `NULL_ACCOUNT`.
pub const NULL_ACCOUNT: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";

/// SEP-41 transfer method name.
pub const TRANSFER_FUNCTION: &str = "transfer";

/// `ledgerTimeout = ceil(maxTimeoutSeconds / estimatedLedgerSeconds)`.
#[must_use]
pub fn timeout_ledgers(max_timeout_seconds: u64, estimated_ledger_seconds: u64) -> u32 {
    let estimated = estimated_ledger_seconds.max(1);
    let ledgers = max_timeout_seconds.div_ceil(estimated);
    u32::try_from(ledgers).unwrap_or(u32::MAX)
}

/// Default Soroban RPC for testnet.
pub const STELLAR_TESTNET_RPC_URL: &str = "https://soroban-testnet.stellar.org";

/// Horizon API for testnet.
pub const STELLAR_TESTNET_HORIZON_URL: &str = "https://horizon-testnet.stellar.org";

/// Horizon API for pubnet.
pub const STELLAR_PUBNET_HORIZON_URL: &str = "https://horizon.stellar.org";

/// Pubnet network passphrase.
pub const STELLAR_PUBNET_PASSPHRASE: &str = "Public Global Stellar Network ; September 2015";

/// Testnet network passphrase.
pub const STELLAR_TESTNET_PASSPHRASE: &str = "Test SDF Network ; September 2015";

/// A Stellar chain reference (`pubnet` or `testnet`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum StellarChainReference {
    /// Stellar pubnet (`stellar:pubnet`).
    Pubnet,
    /// Stellar testnet (`stellar:testnet`).
    Testnet,
}

impl StellarChainReference {
    /// Stellar pubnet (`stellar:pubnet`).
    pub const PUBNET: Self = Self::Pubnet;

    /// Stellar testnet (`stellar:testnet`).
    pub const TESTNET: Self = Self::Testnet;

    /// All chain references with built-in support.
    pub const ALL: &'static [Self] = &[Self::Pubnet, Self::Testnet];

    /// Returns the CAIP-2 reference string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pubnet => "pubnet",
            Self::Testnet => "testnet",
        }
    }

    /// Returns the Stellar network passphrase.
    #[must_use]
    pub const fn passphrase(self) -> &'static str {
        match self {
            Self::Pubnet => STELLAR_PUBNET_PASSPHRASE,
            Self::Testnet => STELLAR_TESTNET_PASSPHRASE,
        }
    }

    /// Returns the default Horizon URL for this network.
    #[must_use]
    pub const fn default_horizon_url(self) -> &'static str {
        match self {
            Self::Pubnet => STELLAR_PUBNET_HORIZON_URL,
            Self::Testnet => STELLAR_TESTNET_HORIZON_URL,
        }
    }

    /// Returns the default Soroban RPC URL, if one exists.
    ///
    /// Pubnet has no public default; the operator must supply an RPC URL.
    #[must_use]
    pub const fn default_rpc_url(self) -> Option<&'static str> {
        match self {
            Self::Pubnet => None,
            Self::Testnet => Some(STELLAR_TESTNET_RPC_URL),
        }
    }

    /// Resolves the RPC URL, requiring an operator URL on pubnet.
    ///
    /// # Errors
    ///
    /// Returns [`StellarRpcUrlError::PubnetRpcRequired`] when `rpc_url` is
    /// empty on pubnet.
    pub fn rpc_url(self, rpc_url: Option<&str>) -> Result<String, StellarRpcUrlError> {
        if let Some(url) = rpc_url.filter(|u| !u.is_empty()) {
            return Ok(url.to_owned());
        }
        match self {
            Self::Testnet => Ok(STELLAR_TESTNET_RPC_URL.to_owned()),
            Self::Pubnet => Err(StellarRpcUrlError::PubnetRpcRequired),
        }
    }
}

impl Debug for StellarChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "StellarChainReference({})", self.as_str())
    }
}

impl Display for StellarChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for StellarChainReference {
    type Err = StellarChainReferenceFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pubnet" => Ok(Self::Pubnet),
            "testnet" => Ok(Self::Testnet),
            other => Err(StellarChainReferenceFormatError::InvalidReference(
                other.to_owned(),
            )),
        }
    }
}

impl Serialize for StellarChainReference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for StellarChainReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<StellarChainReference> for ChainId {
    fn from(value: StellarChainReference) -> Self {
        Self::new(STELLAR_NAMESPACE, value.as_str())
    }
}

impl TryFrom<ChainId> for StellarChainReference {
    type Error = StellarChainReferenceFormatError;

    fn try_from(value: ChainId) -> Result<Self, Self::Error> {
        let (namespace, reference) = value.into_parts();
        if namespace != STELLAR_NAMESPACE {
            return Err(StellarChainReferenceFormatError::InvalidNamespace(
                namespace,
            ));
        }
        Self::from_str(&reference)
            .map_err(|_| StellarChainReferenceFormatError::InvalidReference(reference))
    }
}

/// Error type for parsing Stellar chain references.
#[derive(Debug, thiserror::Error)]
pub enum StellarChainReferenceFormatError {
    /// The namespace was not `"stellar"`.
    #[error("Invalid namespace {0}, expected stellar")]
    InvalidNamespace(String),
    /// The reference was not `pubnet` or `testnet`.
    #[error("Invalid stellar chain reference {0}")]
    InvalidReference(String),
}

/// Error resolving a Stellar RPC URL.
#[derive(Debug, thiserror::Error, Clone, Copy)]
pub enum StellarRpcUrlError {
    /// Pubnet has no public default RPC; the operator must supply one.
    #[error(
        "Stellar pubnet requires a non-empty rpcUrl. For a list of RPC providers, see https://developers.stellar.org/docs/data/apis/rpc/providers#publicly-accessible-apis"
    )]
    PubnetRpcRequired,
}

/// Returns `true` when `network` is a canonical Stellar CAIP-2 identifier.
#[must_use]
pub fn is_stellar_network(network: &str) -> bool {
    network == "stellar:pubnet" || network == "stellar:testnet"
}

/// Ed25519 payload of a G-account or muxed M-account strkey.
///
/// Contract (`C…`) and other strkey types return `None`. Used so facilitator
/// safety checks treat `G…` and `M…` of the same key as the same account.
#[must_use]
pub fn ed25519_account_payload(address: &str) -> Option<[u8; 32]> {
    match Strkey::from_string(address).ok()? {
        Strkey::PublicKeyEd25519(pk) => Some(pk.0),
        Strkey::MuxedAccountEd25519(muxed) => Some(muxed.ed25519),
        _ => None,
    }
}

/// Returns `true` when `candidate` is the same G/M ed25519 key as any
/// facilitator address.
#[must_use]
pub fn is_facilitator_account(facilitator_addresses: &[String], candidate: &str) -> bool {
    ed25519_account_payload(candidate).map_or_else(
        || facilitator_addresses.iter().any(|addr| addr == candidate),
        |key| {
            facilitator_addresses
                .iter()
                .any(|addr| ed25519_account_payload(addr) == Some(key))
        },
    )
}

/// A Stellar address: G-account, C-account, or muxed M-account.
///
/// Stored as [`CompactString`]. Validation uses [`stellar_strkey`].
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct StellarAddress(CompactString);

impl StellarAddress {
    /// Returns the address as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Returns `true` when this is a contract (`C…`) address.
    #[must_use]
    pub fn is_contract(&self) -> bool {
        matches!(Strkey::from_string(self.as_str()), Ok(Strkey::Contract(_)))
    }
}

impl Display for StellarAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for StellarAddress {
    type Err = StellarAddressFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match Strkey::from_string(s) {
            Ok(
                Strkey::PublicKeyEd25519(_) | Strkey::Contract(_) | Strkey::MuxedAccountEd25519(_),
            ) => Ok(Self(CompactString::from(s))),
            Ok(_) | Err(_) => Err(StellarAddressFormatError::Invalid(s.to_owned())),
        }
    }
}

impl Serialize for StellarAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for StellarAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl AsRef<str> for StellarAddress {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// Errors that can occur when parsing a Stellar address.
#[derive(Debug, thiserror::Error)]
pub enum StellarAddressFormatError {
    /// The string is not a G, C, or M Stellar address.
    #[error("invalid stellar address: {0}")]
    Invalid(String),
}

/// Parses a SEP-41 contract address (`C…` only).
///
/// # Errors
///
/// Returns [`StellarAddressFormatError`] when the string is not a contract.
pub fn parse_contract_address(s: &str) -> Result<StellarAddress, StellarAddressFormatError> {
    let address: StellarAddress = s.parse()?;
    if address.is_contract() {
        Ok(address)
    } else {
        Err(StellarAddressFormatError::Invalid(s.to_owned()))
    }
}

/// SEP-41 atomic units as a decimal string; parsed as `i128`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StellarTokenAmount(CompactString);

impl StellarTokenAmount {
    /// Returns the decimal string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Parses the amount as `i128`.
    ///
    /// # Errors
    ///
    /// Returns [`StellarTokenAmountFormatError`] if the string is not a decimal `i128`.
    pub fn as_i128(&self) -> Result<i128, StellarTokenAmountFormatError> {
        self.0
            .parse()
            .map_err(|_| StellarTokenAmountFormatError::Invalid(self.0.to_string()))
    }
}

impl Display for StellarTokenAmount {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for StellarTokenAmount {
    type Err = StellarTokenAmountFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty()
            || !(s.bytes().all(|b| b.is_ascii_digit())
                || (s.starts_with('-')
                    && s.len() > 1
                    && s.bytes().skip(1).all(|b| b.is_ascii_digit())))
        {
            return Err(StellarTokenAmountFormatError::Invalid(s.to_owned()));
        }
        let _: i128 = s
            .parse()
            .map_err(|_| StellarTokenAmountFormatError::Invalid(s.to_owned()))?;
        Ok(Self(CompactString::from(s)))
    }
}

impl Serialize for StellarTokenAmount {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for StellarTokenAmount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<i128> for StellarTokenAmount {
    fn from(value: i128) -> Self {
        Self(CompactString::from(value.to_string()))
    }
}

impl From<u128> for StellarTokenAmount {
    fn from(value: u128) -> Self {
        Self(CompactString::from(value.to_string()))
    }
}

impl TryFrom<StellarTokenAmount> for i128 {
    type Error = StellarTokenAmountFormatError;

    fn try_from(value: StellarTokenAmount) -> Result<Self, Self::Error> {
        value.as_i128()
    }
}

/// Error parsing a SEP-41 token amount.
#[derive(Debug, thiserror::Error)]
pub enum StellarTokenAmountFormatError {
    /// The string is not a signed decimal integer.
    #[error("invalid stellar token amount: {0}")]
    Invalid(String),
}

/// Information about a SEP-41 token deployment on a Stellar network.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StellarTokenDeployment {
    /// The Stellar network where this token is deployed.
    pub chain_reference: StellarChainReference,
    /// The SEP-41 contract address (`C…`).
    pub address: StellarAddress,
    /// The number of decimal places for this token.
    pub decimals: u8,
}

impl StellarTokenDeployment {
    /// Creates a new token deployment.
    #[must_use]
    pub const fn new(
        chain_reference: StellarChainReference,
        address: StellarAddress,
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
    pub fn amount(&self, v: i128) -> DeployedTokenAmount<i128, Self> {
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
    pub fn parse<V>(&self, v: V) -> Result<DeployedTokenAmount<i128, Self>, MoneyAmountParseError>
    where
        V: TryInto<MoneyAmount>,
        MoneyAmountParseError: From<<V as TryInto<MoneyAmount>>::Error>,
    {
        let amount: u128 = v.try_into()?.to_token_amount(self.decimals)?;
        let amount = i128::try_from(amount).map_err(|_| MoneyAmountParseError::OutOfRange)?;
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
    fn destination_and_contract_addresses() {
        assert!(
            "GBBO4ZDDZTSM2IUKQYBAST3CFHNPFXECGEFTGWTA2WELR2BIWDK57UVE"
                .parse::<StellarAddress>()
                .is_ok()
        );
        assert!(
            "CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA"
                .parse::<StellarAddress>()
                .is_ok()
        );
        assert!("not-an-address".parse::<StellarAddress>().is_err());
        assert!(
            parse_contract_address("CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA")
                .is_ok()
        );
        assert!(
            parse_contract_address("GBBO4ZDDZTSM2IUKQYBAST3CFHNPFXECGEFTGWTA2WELR2BIWDK57UVE")
                .is_err()
        );
    }

    #[test]
    fn token_amount_decimal_string() {
        let amount: StellarTokenAmount = "10000000".parse().unwrap();
        assert_eq!(amount.as_i128().unwrap(), 10_000_000);
        assert!("".parse::<StellarTokenAmount>().is_err());
        assert!("1.0".parse::<StellarTokenAmount>().is_err());
    }

    #[test]
    fn chain_reference_roundtrip() {
        let chain_id: ChainId = StellarChainReference::TESTNET.into();
        assert_eq!(chain_id.to_string(), "stellar:testnet");
        let back = StellarChainReference::try_from(chain_id).unwrap();
        assert_eq!(back, StellarChainReference::TESTNET);
        assert!(is_stellar_network("stellar:pubnet"));
        assert!(!is_stellar_network("eip155:1"));
        let g = "GBBO4ZDDZTSM2IUKQYBAST3CFHNPFXECGEFTGWTA2WELR2BIWDK57UVE";
        let payload = ed25519_account_payload(g).unwrap();
        let muxed = format!(
            "{}",
            stellar_strkey::ed25519::MuxedAccount {
                ed25519: payload,
                id: 7,
            }
        );
        assert_eq!(ed25519_account_payload(&muxed), Some(payload));
        assert!(is_facilitator_account(&[g.to_owned()], &muxed));
        assert!(!is_facilitator_account(
            &[g.to_owned()],
            "GCQAXB2D77Y4C66CTGVH25H2RMUKMQJGOWUPK7UXGG5MAQBONUEKFQ4P"
        ));
        assert!(StellarChainReference::PUBNET.rpc_url(None).is_err());
        assert!(
            StellarChainReference::PUBNET
                .rpc_url(Some("https://rpc.example"))
                .is_ok()
        );
        assert_eq!(
            StellarChainReference::TESTNET.passphrase(),
            STELLAR_TESTNET_PASSPHRASE
        );
    }

    #[test]
    fn token_deployment_parse() {
        let addr: StellarAddress = "CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA"
            .parse()
            .unwrap();
        let deployment = StellarTokenDeployment::new(StellarChainReference::TESTNET, addr, 7);
        let parsed = deployment.parse("10.50").unwrap();
        assert_eq!(parsed.amount, 105_000_000);
    }
}
