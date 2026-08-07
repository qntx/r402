//! Wire format types for Tron chain interactions.
//!
//! This module provides types that handle serialization and deserialization
//! of Tron-specific values in the x402 protocol wire format.

use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;

use alloy_primitives::{Address as EvmAddress, U256};
use r402_core::amount::{MoneyAmount, MoneyAmountParseError, ScaleFromMantissa};
use r402_core::chain::{ChainId, DeployedTokenAmount};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

/// The CAIP-2 namespace for Tron chains.
pub const TRON_NAMESPACE: &str = "tron";

/// Tron mainnet address version byte prepended before `Base58Check` encoding.
const ADDRESS_PREFIX: u8 = 0x41;

/// A Tron chain reference: the last 4 bytes of the genesis block hash (TIP-474).
///
/// Serialized as a lowercase `0x`-prefixed 8-digit hex string (e.g.
/// `"0x2b6653dc"`) per the CAIP-2 convention adopted for the `tron`
/// namespace.
///
/// # Well-Known References
///
/// - Mainnet: `tron:0x2b6653dc`
/// - Nile testnet: `tron:0xcd8690dc`
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TronChainReference(u32);

impl TronChainReference {
    /// Tron mainnet (`tron:0x2b6653dc`).
    pub const MAINNET: Self = Self::new(0x2b66_53dc);

    /// Tron Nile testnet (`tron:0xcd8690dc`).
    pub const NILE: Self = Self::new(0xcd86_90dc);

    /// Creates a new chain reference from a raw chain ID.
    #[must_use]
    pub const fn new(chain_id: u32) -> Self {
        Self(chain_id)
    }

    /// Returns the raw numeric chain ID.
    #[must_use]
    pub const fn inner(self) -> u32 {
        self.0
    }
}

impl Debug for TronChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "TronChainReference({self})")
    }
}

impl Display for TronChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{:08x}", self.0)
    }
}

impl FromStr for TronChainReference {
    type Err = TronChainReferenceFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s
            .strip_prefix("0x")
            .or_else(|| s.strip_prefix("0X"))
            .ok_or_else(|| TronChainReferenceFormatError::InvalidReference(s.to_owned()))?;
        let value = u32::from_str_radix(hex, 16)
            .map_err(|_| TronChainReferenceFormatError::InvalidReference(s.to_owned()))?;
        Ok(Self(value))
    }
}

impl Serialize for TronChainReference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for TronChainReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<TronChainReference> for ChainId {
    fn from(value: TronChainReference) -> Self {
        Self::new(TRON_NAMESPACE, value.to_string())
    }
}

impl TryFrom<ChainId> for TronChainReference {
    type Error = TronChainReferenceFormatError;

    fn try_from(value: ChainId) -> Result<Self, Self::Error> {
        let (namespace, reference) = value.into_parts();
        if namespace != TRON_NAMESPACE {
            return Err(TronChainReferenceFormatError::InvalidNamespace(namespace));
        }
        Self::from_str(&reference)
            .map_err(|_| TronChainReferenceFormatError::InvalidReference(reference))
    }
}

/// Error type for parsing Tron chain references.
#[derive(Debug, thiserror::Error)]
pub enum TronChainReferenceFormatError {
    /// The namespace was not "tron".
    #[error("Invalid namespace {0}, expected tron")]
    InvalidNamespace(String),
    /// The reference was not a valid `0x`-prefixed hex string.
    #[error("Invalid tron chain reference {0}")]
    InvalidReference(String),
}

/// A Tron account address.
///
/// Tron addresses share the same 20-byte secp256k1-derived payload as
/// Ethereum addresses, but are conventionally encoded on the wire (and in
/// `TronGrid` API calls) using `Base58Check` with a `0x41` version byte prefix
/// and a 4-byte double-SHA256 checksum, producing the familiar `T...`
/// representation.
///
/// At the TIP-712 (EIP-712-compatible) signing layer, Tron reuses the raw
/// 20-byte EVM address form — see [`Self::as_evm`] / [`Self::from_evm`] for
/// the conversion used when constructing typed-data payloads.
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct Address([u8; 20]);

impl Address {
    /// Creates an address from raw 20 bytes (the shared EVM/Tron key payload).
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    /// Returns the underlying 20-byte payload.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }

    /// Converts to the raw EVM hex address used at the TIP-712 signing layer.
    #[must_use]
    pub const fn as_evm(&self) -> EvmAddress {
        EvmAddress::new(self.0)
    }

    /// Constructs a Tron address from a raw EVM hex address.
    #[must_use]
    pub const fn from_evm(address: EvmAddress) -> Self {
        Self(address.into_array())
    }

    /// Decodes a Base58Check-encoded Tron address (the `T...` wire format).
    ///
    /// # Errors
    ///
    /// Returns [`AddressFormatError`] if the string is not valid Base58, has
    /// the wrong decoded length, fails the checksum, or lacks the `0x41`
    /// Tron address prefix.
    #[allow(
        clippy::indexing_slicing,
        reason = "checksum comparison over a fixed-size digest output, length pre-validated"
    )]
    pub fn from_base58check(s: &str) -> Result<Self, AddressFormatError> {
        let decoded = bs58::decode(s)
            .into_vec()
            .map_err(|_| AddressFormatError::InvalidBase58(s.to_owned()))?;
        if decoded.len() != 25 {
            return Err(AddressFormatError::InvalidLength(decoded.len()));
        }
        let (payload, checksum) = decoded.split_at(21);
        let &[prefix, ref address_bytes @ ..] = payload else {
            return Err(AddressFormatError::InvalidLength(decoded.len()));
        };
        if prefix != ADDRESS_PREFIX {
            return Err(AddressFormatError::InvalidPrefix(prefix));
        }
        let round1 = Sha256::digest(payload);
        let round2 = Sha256::digest(round1);
        if checksum != &round2[..4] {
            return Err(AddressFormatError::InvalidChecksum);
        }
        let mut bytes = [0u8; 20];
        bytes.copy_from_slice(address_bytes);
        Ok(Self(bytes))
    }

    /// Encodes the address as `Base58Check` (the `T...` wire format).
    #[must_use]
    #[allow(
        clippy::indexing_slicing,
        reason = "fixed-size local buffers with statically known layout"
    )]
    pub fn to_base58check(self) -> String {
        let mut payload = [0u8; 21];
        payload[0] = ADDRESS_PREFIX;
        payload[1..].copy_from_slice(&self.0);
        let round1 = Sha256::digest(payload);
        let round2 = Sha256::digest(round1);
        let mut full = [0u8; 25];
        full[..21].copy_from_slice(&payload);
        full[21..].copy_from_slice(&round2[..4]);
        bs58::encode(full).into_string()
    }
}

/// Errors that can occur when parsing a Base58Check-encoded Tron address.
#[derive(Debug, thiserror::Error)]
pub enum AddressFormatError {
    /// The string was not valid Base58.
    #[error("invalid base58 tron address: {0}")]
    InvalidBase58(String),
    /// The decoded payload was not 25 bytes (1 prefix + 20 address + 4 checksum).
    #[error("invalid tron address length: expected 25 bytes, got {0}")]
    InvalidLength(usize),
    /// The double-SHA256 checksum did not match.
    #[error("invalid tron address checksum")]
    InvalidChecksum,
    /// The version byte was not `0x41`.
    #[error("invalid tron address prefix: expected 0x41, got 0x{0:02x}")]
    InvalidPrefix(u8),
}

impl Debug for Address {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Address")
            .field(&self.to_base58check())
            .finish()
    }
}

impl Display for Address {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_base58check())
    }
}

impl FromStr for Address {
    type Err = AddressFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_base58check(s)
    }
}

impl Serialize for Address {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_base58check())
    }
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_base58check(&s).map_err(serde::de::Error::custom)
    }
}

impl From<EvmAddress> for Address {
    fn from(address: EvmAddress) -> Self {
        Self::from_evm(address)
    }
}

impl From<Address> for EvmAddress {
    fn from(address: Address) -> Self {
        address.as_evm()
    }
}

/// A token amount represented as a `U256`, serialized as a decimal string.
///
/// TIP-712 typed data (EIP-3009 `value`, Permit2 `amount`/`nonce`/`deadline`)
/// carries `uint256` fields, so — unlike SVM's `u64` lamport amounts — Tron
/// token amounts use the same 256-bit representation as EVM.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TronTokenAmount(pub U256);

impl FromStr for TronTokenAmount {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let u256 = U256::from_str_radix(s, 10).map_err(|_| "invalid token amount".to_owned())?;
        Ok(Self(u256))
    }
}

impl Serialize for TronTokenAmount {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for TronTokenAmount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).map_err(serde::de::Error::custom)
    }
}

impl From<TronTokenAmount> for U256 {
    fn from(value: TronTokenAmount) -> Self {
        value.0
    }
}

impl From<U256> for TronTokenAmount {
    fn from(value: U256) -> Self {
        Self(value)
    }
}

impl ScaleFromMantissa for TronTokenAmount {
    fn from_mantissa_scaled(
        mantissa: u128,
        scale_diff: u32,
    ) -> Result<Self, MoneyAmountParseError> {
        let multiplier = U256::from(10)
            .checked_pow(U256::from(scale_diff))
            .ok_or(MoneyAmountParseError::OutOfRange)?;
        let value = U256::from(mantissa)
            .checked_mul(multiplier)
            .ok_or(MoneyAmountParseError::OutOfRange)?;
        Ok(Self(value))
    }
}

/// TIP-712 domain parameters for a token deployment.
///
/// Used when constructing typed-data signatures for EIP-3009
/// `transferWithAuthorization` calls (Permit2 payments sign against the
/// fixed `"Permit2"` domain instead, with no per-token name/version).
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct TokenDeploymentTip712 {
    /// The token name as specified in the TIP-712 domain.
    pub name: String,
    /// The token version as specified in the TIP-712 domain.
    pub version: String,
}

/// Information about a TRC-20 token deployment on a Tron network.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TronTokenDeployment {
    /// The Tron network where this token is deployed.
    pub chain_reference: TronChainReference,
    /// The TRC-20 contract address.
    pub address: Address,
    /// The number of decimal places for this token.
    pub decimals: u8,
    /// Optional TIP-712 domain parameters (required for the EIP-3009 transfer method).
    pub tip712: Option<TokenDeploymentTip712>,
}

impl TronTokenDeployment {
    /// Creates a new token deployment with no TIP-712 domain (Permit2-only).
    #[must_use]
    pub const fn new(chain_reference: TronChainReference, address: Address, decimals: u8) -> Self {
        Self {
            chain_reference,
            address,
            decimals,
            tip712: None,
        }
    }

    /// Builder: attaches TIP-712 domain parameters (required for EIP-3009).
    #[must_use]
    pub fn with_tip712(mut self, name: impl Into<String>, version: impl Into<String>) -> Self {
        self.tip712 = Some(TokenDeploymentTip712 {
            name: name.into(),
            version: version.into(),
        });
        self
    }

    /// Creates a deployed token amount from a raw value in the token's
    /// smallest unit (e.g. `1_000_000` for 1 USDT at 6 decimals).
    pub fn amount<V: Into<TronTokenAmount>>(&self, v: V) -> DeployedTokenAmount<U256, Self> {
        DeployedTokenAmount {
            amount: v.into().0,
            token: self.clone(),
        }
    }

    /// Parses a human-readable amount into a deployed token amount.
    ///
    /// Accepts formats like `"10.50"`, `"$10.50"`, `"1,000"`.
    ///
    /// # Errors
    ///
    /// Returns [`MoneyAmountParseError`] if the value cannot be parsed, exceeds precision,
    /// or overflows `U256`.
    pub fn parse<V>(&self, v: V) -> Result<DeployedTokenAmount<U256, Self>, MoneyAmountParseError>
    where
        V: TryInto<MoneyAmount>,
        MoneyAmountParseError: From<<V as TryInto<MoneyAmount>>::Error>,
    {
        let TronTokenAmount(amount) = v.try_into()?.to_token_amount(self.decimals)?;
        Ok(DeployedTokenAmount {
            amount,
            token: self.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// USDT on Tron mainnet — canonical `Base58Check` test vector.
    const USDT_MAINNET: &str = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";

    #[test]
    fn base58check_roundtrip() {
        let addr = Address::from_base58check(USDT_MAINNET).unwrap();
        assert_eq!(addr.to_base58check(), USDT_MAINNET);
    }

    #[test]
    fn evm_roundtrip() {
        let addr = Address::from_base58check(USDT_MAINNET).unwrap();
        let evm = addr.as_evm();
        assert_eq!(Address::from_evm(evm), addr);
    }

    #[test]
    fn rejects_bad_checksum() {
        // Flip the last character of a valid address to corrupt the checksum.
        let mut corrupted = USDT_MAINNET.to_owned();
        corrupted.pop();
        corrupted.push('1');
        assert!(matches!(
            Address::from_base58check(&corrupted),
            Err(AddressFormatError::InvalidChecksum | AddressFormatError::InvalidBase58(_))
        ));
    }

    #[test]
    fn rejects_wrong_prefix() {
        // Re-encode the mainnet USDT payload with an invalid prefix byte.
        let addr = Address::from_base58check(USDT_MAINNET).unwrap();
        let mut payload = [0u8; 21];
        payload[0] = 0x00;
        payload[1..].copy_from_slice(addr.as_bytes());
        let round1 = Sha256::digest(payload);
        let round2 = Sha256::digest(round1);
        let mut full = [0u8; 25];
        full[..21].copy_from_slice(&payload);
        full[21..].copy_from_slice(round2.get(..4).expect("sha256 output >= 4 bytes"));
        let encoded = bs58::encode(full).into_string();
        assert!(matches!(
            Address::from_base58check(&encoded),
            Err(AddressFormatError::InvalidPrefix(0x00))
        ));
    }

    #[test]
    fn chain_reference_roundtrip() {
        let chain_id: ChainId = TronChainReference::MAINNET.into();
        assert_eq!(chain_id.to_string(), "tron:0x2b6653dc");
        let back = TronChainReference::try_from(chain_id).unwrap();
        assert_eq!(back, TronChainReference::MAINNET);
    }

    #[test]
    fn chain_reference_rejects_wrong_namespace() {
        let chain_id = ChainId::new("eip155", "1");
        assert!(matches!(
            TronChainReference::try_from(chain_id),
            Err(TronChainReferenceFormatError::InvalidNamespace(_))
        ));
    }

    #[test]
    fn token_deployment_parse() {
        let deployment = TronTokenDeployment::new(
            TronChainReference::MAINNET,
            Address::from_base58check(USDT_MAINNET).unwrap(),
            6,
        );
        let parsed = deployment.parse("10.50").unwrap();
        assert_eq!(parsed.amount, U256::from(10_500_000u64));
    }
}
