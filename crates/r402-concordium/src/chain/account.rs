//! Concordium addresses, CAIP-2 chain references, and token deployments.

use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;

use r402_protocol::money::{MoneyAmount, MoneyAmountParseError, ScaleFromMantissa};
use r402_protocol::network::{ChainId, DeployedTokenAmount};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// CAIP-2 namespace for Concordium.
pub const CONCORDIUM_NAMESPACE: &str = "ccd";

/// Mainnet genesis-hash CAIP-2 reference.
pub const CONCORDIUM_MAINNET_CAIP_REF: &str = "9dd9ca4d19e9393877d2c44b70f89acb";

/// Testnet genesis-hash CAIP-2 reference.
pub const CONCORDIUM_TESTNET_CAIP_REF: &str = "4221332d34e1694168c2a0c0b3fd0f27";

/// Mainnet CAIP-2 identifier.
pub const CONCORDIUM_MAINNET_CAIP2: &str = "ccd:9dd9ca4d19e9393877d2c44b70f89acb";

/// Testnet CAIP-2 identifier.
pub const CONCORDIUM_TESTNET_CAIP2: &str = "ccd:4221332d34e1694168c2a0c0b3fd0f27";

/// Wildcard matching all Concordium networks.
pub const CONCORDIUM_WILDCARD_CAIP2: &str = "ccd:*";

/// Default mainnet gRPC endpoint (`host:port`).
pub const CONCORDIUM_MAINNET_GRPC: &str = "grpc.mainnet.concordium.software:20000";

/// Default testnet gRPC endpoint (`host:port`).
pub const CONCORDIUM_TESTNET_GRPC: &str = "grpc.testnet.concordium.com:20000";

/// Default gRPC port when the endpoint omits one.
pub const DEFAULT_GRPC_PORT: u16 = 20_000;

/// Mainnet explorer base URL.
pub const CONCORDIUM_MAINNET_EXPLORER: &str = "https://ccdexplorer.io/mainnet";

/// Testnet explorer base URL.
pub const CONCORDIUM_TESTNET_EXPLORER: &str = "https://ccdexplorer.io/testnet";

/// Native CCD decimals (microCCD).
pub const CCD_DECIMALS: u8 = 6;

/// Native CCD asset identifier.
pub const CCD_ASSET_IDENTIFIER: &str = "CCD";

/// `StablR` USDR token id (mainnet and testnet).
pub const USDR_TOKEN_ID: &str = "USDR";

/// USDR decimals.
pub const USDR_DECIMALS: u8 = 6;

/// Spec Rule 7 default max expiry offset.
pub const MAX_EXPIRY_OFFSET_SECONDS: u64 = 600;

/// Default `ConcordiumBFT` finalization wait.
pub const DEFAULT_FINALIZATION_TIMEOUT_MS: u64 = 60_000;

/// Base58 alphabet without `0`/`O`/`I`/`l`, 45–55 chars.
pub const CONCORDIUM_ADDRESS_PATTERN: &str =
    r"^[123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz]{45,55}$";

const ADDRESS_VERSION: u8 = 1;
const ADDRESS_SIZE: usize = 32;

/// A Concordium chain reference (mainnet or testnet).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConcordiumChainReference {
    caip_ref: &'static str,
    grpc: &'static str,
    explorer: &'static str,
}

impl ConcordiumChainReference {
    /// Concordium mainnet.
    pub const MAINNET: Self = Self {
        caip_ref: CONCORDIUM_MAINNET_CAIP_REF,
        grpc: CONCORDIUM_MAINNET_GRPC,
        explorer: CONCORDIUM_MAINNET_EXPLORER,
    };

    /// Concordium testnet.
    pub const TESTNET: Self = Self {
        caip_ref: CONCORDIUM_TESTNET_CAIP_REF,
        grpc: CONCORDIUM_TESTNET_GRPC,
        explorer: CONCORDIUM_TESTNET_EXPLORER,
    };

    /// Built-in networks.
    pub const ALL: &'static [Self] = &[Self::MAINNET, Self::TESTNET];

    /// CAIP-2 reference (genesis hash hex).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.caip_ref
    }

    /// Default gRPC `host:port` for this network.
    #[must_use]
    pub const fn default_grpc(self) -> &'static str {
        self.grpc
    }

    /// Explorer base URL.
    #[must_use]
    pub const fn explorer_base(self) -> &'static str {
        self.explorer
    }

    /// HTTPS URI for the official gRPC v2 client.
    #[must_use]
    pub fn default_grpc_https(self) -> String {
        format!("https://{}", self.grpc)
    }
}

impl Debug for ConcordiumChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "ConcordiumChainReference({})", self.as_str())
    }
}

impl Display for ConcordiumChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ConcordiumChainReference {
    type Err = ConcordiumChainReferenceFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_concordium_reference(s)
            .ok_or_else(|| ConcordiumChainReferenceFormatError::InvalidReference(s.to_owned()))
    }
}

impl Serialize for ConcordiumChainReference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ConcordiumChainReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<ConcordiumChainReference> for ChainId {
    fn from(value: ConcordiumChainReference) -> Self {
        Self::new(CONCORDIUM_NAMESPACE, value.as_str())
    }
}

impl TryFrom<ChainId> for ConcordiumChainReference {
    type Error = ConcordiumChainReferenceFormatError;

    fn try_from(value: ChainId) -> Result<Self, Self::Error> {
        let (namespace, reference) = value.into_parts();
        if namespace != CONCORDIUM_NAMESPACE {
            return Err(ConcordiumChainReferenceFormatError::InvalidNamespace(
                namespace,
            ));
        }
        Self::from_str(&reference)
    }
}

/// Error parsing a Concordium CAIP-2 reference.
#[derive(Debug, thiserror::Error)]
pub enum ConcordiumChainReferenceFormatError {
    /// Namespace was not `"ccd"`.
    #[error("Invalid namespace {0}, expected ccd")]
    InvalidNamespace(String),
    /// Reference was not a known Concordium genesis hash.
    #[error("Invalid concordium chain reference {0}")]
    InvalidReference(String),
}

/// Maps a CAIP-2 identifier to a known Concordium network.
#[must_use]
pub fn normalize_concordium_network(network: &str) -> Option<ConcordiumChainReference> {
    let reference = network.strip_prefix("ccd:")?;
    parse_concordium_reference(reference)
}

fn parse_concordium_reference(reference: &str) -> Option<ConcordiumChainReference> {
    if reference == CONCORDIUM_MAINNET_CAIP_REF {
        Some(ConcordiumChainReference::MAINNET)
    } else if reference == CONCORDIUM_TESTNET_CAIP_REF {
        Some(ConcordiumChainReference::TESTNET)
    } else {
        None
    }
}

/// gRPC endpoint for `network`, or an error if the CAIP-2 id is unknown.
///
/// # Errors
///
/// Returns [`ConcordiumChainReferenceFormatError`] when `network` is not
/// mainnet or testnet.
pub fn get_concordium_grpc_url(
    network: &str,
) -> Result<&'static str, ConcordiumChainReferenceFormatError> {
    normalize_concordium_network(network)
        .map(ConcordiumChainReference::default_grpc)
        .ok_or_else(|| ConcordiumChainReferenceFormatError::InvalidReference(network.to_owned()))
}

/// Explorer URL for a transaction hash, if the network is known.
#[must_use]
pub fn get_explorer_tx_url(network: &str, tx_hash: &str) -> Option<String> {
    let chain = normalize_concordium_network(network)?;
    Some(format!("{}/transaction/{tx_hash}", chain.explorer_base()))
}

/// Explorer URL for an account, if the network is known.
#[must_use]
pub fn get_explorer_account_url(network: &str, address: &str) -> Option<String> {
    let chain = normalize_concordium_network(network)?;
    Some(format!("{}/account/{address}", chain.explorer_base()))
}

/// Parses `host:port`. Missing or non-numeric port becomes [`DEFAULT_GRPC_PORT`].
#[must_use]
pub fn parse_grpc_url(grpc_url: &str) -> (&str, u16) {
    match grpc_url.rsplit_once(':') {
        Some((host, port_str)) => {
            let port = port_str.parse().unwrap_or(DEFAULT_GRPC_PORT);
            (host, port)
        }
        None => (grpc_url, DEFAULT_GRPC_PORT),
    }
}

/// Structural (alphabet + length) check used by the official regex tests.
#[must_use]
pub fn address_matches_regex(address: &str) -> bool {
    let len = address.len();
    if !(45..=55).contains(&len) {
        return false;
    }
    address.bytes().all(|b| {
        matches!(
            b,
            b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z'
        )
    })
}

/// A Concordium account address (32 bytes, `Base58Check` version 1).
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct ConcordiumAddress([u8; ADDRESS_SIZE]);

impl ConcordiumAddress {
    /// Constructs an address from a 32-byte payload.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; ADDRESS_SIZE]) -> Self {
        Self(bytes)
    }

    /// Decodes a `Base58Check` version-1 account address.
    ///
    /// # Errors
    ///
    /// Returns [`ConcordiumAddressFormatError`] when checksum, version, or
    /// length is wrong.
    pub fn decode(s: &str) -> Result<Self, ConcordiumAddressFormatError> {
        let mut buf = [0u8; ADDRESS_SIZE + 4 + 1];
        let len = bs58::decode(s)
            .with_check(Some(ADDRESS_VERSION))
            .onto(&mut buf)
            .map_err(ConcordiumAddressFormatError::InvalidBase58Check)?;
        if len != 1 + ADDRESS_SIZE {
            return Err(ConcordiumAddressFormatError::InvalidByteLength(len));
        }
        let Some(payload) = buf.get(1..1 + ADDRESS_SIZE) else {
            return Err(ConcordiumAddressFormatError::InvalidByteLength(len));
        };
        let mut bytes = [0u8; ADDRESS_SIZE];
        bytes.copy_from_slice(payload);
        Ok(Self(bytes))
    }

    /// 32-byte account payload.
    #[must_use]
    pub const fn as_bytes(self) -> [u8; ADDRESS_SIZE] {
        self.0
    }
}

impl Debug for ConcordiumAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "ConcordiumAddress({self})")
    }
}

impl Display for ConcordiumAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let encoded = bs58::encode(self.0)
            .with_check_version(ADDRESS_VERSION)
            .into_string();
        f.write_str(&encoded)
    }
}

impl FromStr for ConcordiumAddress {
    type Err = ConcordiumAddressFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::decode(s)
    }
}

impl Serialize for ConcordiumAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ConcordiumAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Error parsing a Concordium account address.
#[derive(Debug, thiserror::Error)]
#[allow(
    missing_copy_implementations,
    reason = "bs58::decode::Error is not Copy on all versions"
)]
pub enum ConcordiumAddressFormatError {
    /// `Base58Check` decoding failed.
    #[error("Invalid Base58Check encoding: {0}")]
    InvalidBase58Check(bs58::decode::Error),
    /// Decoded payload was not 1 + 32 bytes.
    #[error("Invalid number of bytes, expected 33, but got {0}.")]
    InvalidByteLength(usize),
}

/// Token deployment on a Concordium network (`CCD` or a PLT symbol).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConcordiumTokenDeployment {
    /// Network this asset is advertised on.
    pub chain_reference: ConcordiumChainReference,
    /// Asset identifier (`CCD`, `USDR`, …).
    pub asset: &'static str,
    /// Decimal places.
    pub decimals: u8,
}

impl ConcordiumTokenDeployment {
    /// Constructs a deployment.
    #[must_use]
    pub const fn new(
        chain_reference: ConcordiumChainReference,
        asset: &'static str,
        decimals: u8,
    ) -> Self {
        Self {
            chain_reference,
            asset,
            decimals,
        }
    }

    /// Amount in atomic units.
    #[must_use]
    pub const fn amount(self, v: u64) -> DeployedTokenAmount<u64, Self> {
        DeployedTokenAmount {
            amount: v,
            token: self,
        }
    }

    /// Parses a human-readable amount into atomic units.
    ///
    /// # Errors
    ///
    /// Returns [`MoneyAmountParseError`] when the string is not a non-negative
    /// decimal or has more fractional digits than `decimals`.
    pub fn parse_amount(
        &self,
        human: &str,
    ) -> Result<DeployedTokenAmount<u64, Self>, MoneyAmountParseError> {
        let money = MoneyAmount::parse(human)?;
        if money.scale() > u32::from(self.decimals) {
            return Err(MoneyAmountParseError::WrongPrecision {
                money: money.scale(),
                token: u32::from(self.decimals),
            });
        }
        let raw = u64::from_mantissa_scaled(
            money.mantissa(),
            u32::from(self.decimals).saturating_sub(money.scale()),
        )?;
        Ok(self.amount(raw))
    }
}

/// Whether `asset` is native CCD (empty, missing, or `"CCD"` ignoring case).
#[must_use]
pub const fn is_native_ccd(asset: &str) -> bool {
    asset.is_empty() || asset.eq_ignore_ascii_case(CCD_ASSET_IDENTIFIER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_regex_accepts_spec_examples() {
        assert!(address_matches_regex(
            "4FmiTW2L4RvCsSVTjFAavYvrgnPLGNj43eiwPYmbhNqtAcMbWW"
        ));
        assert!(address_matches_regex(
            "3kBx2h5Y2veb4hZvAE2c1Zr6DYJwWbPr9xQJJBPWyFnXHF9UuN"
        ));
    }

    #[test]
    fn address_regex_rejects_forbidden_alphabet() {
        assert!(!address_matches_regex(
            "0FmiTW2L4RvCsSVTjFAavYvrgnPLGNj43eiwPYmbhNqtAcMbWW"
        ));
        assert!(!address_matches_regex(
            "OFmiTW2L4RvCsSVTjFAavYvrgnPLGNj43eiwPYmbhNqtAcMbWW"
        ));
        assert!(!address_matches_regex(
            "IFmiTW2L4RvCsSVTjFAavYvrgnPLGNj43eiwPYmbhNqtAcMbWW"
        ));
        assert!(!address_matches_regex(
            "lFmiTW2L4RvCsSVTjFAavYvrgnPLGNj43eiwPYmbhNqtAcMbWW"
        ));
        assert!(!address_matches_regex("4Fmi"));
        assert!(!address_matches_regex(&"a".repeat(56)));
        assert!(!address_matches_regex(""));
    }

    #[test]
    fn spec_addresses_checksum() {
        let known: ConcordiumAddress = "3UrcxPQeYywasrPcYUcqhvFu3SB2vBBDjj7TsaRQ431vGiczYp"
            .parse()
            .expect("known-good checksum");
        assert_eq!(
            known.to_string(),
            "3UrcxPQeYywasrPcYUcqhvFu3SB2vBBDjj7TsaRQ431vGiczYp"
        );
        let generated = ConcordiumAddress::from_bytes([1u8; 32]);
        let encoded = generated.to_string();
        let back: ConcordiumAddress = encoded.parse().expect("roundtrip");
        assert_eq!(back, generated);
    }
}
