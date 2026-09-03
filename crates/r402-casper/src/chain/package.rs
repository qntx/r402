//! Casper network references and CEP-18 package hashes.
//!
//! A package hash is untagged (the x402 `asset`). A chain reference is the
//! CAIP-2 `casper:<chain_name>` bound into every signed deploy.

use std::fmt::{self, Debug, Display, Formatter};
use std::str::FromStr;

use r402_protocol::network::{ChainId, DeployedTokenAmount};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::codec::{self, HASH_HEX_LEN, HexDecodeError};
use super::motes::{CSPR_DECIMALS, Motes, MotesParseError};

/// The CAIP-2 namespace for Casper chains.
pub const CASPER_NAMESPACE: &str = "casper";

/// A Casper chain reference.
///
/// Casper identifies networks by chain name, so the CAIP-2 reference is the
/// chain name itself (`casper:casper`, `casper:casper-test`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum CasperChainReference {
    /// Casper mainnet (`casper:casper`).
    Mainnet,
    /// Casper testnet (`casper:casper-test`).
    Testnet,
}

impl CasperChainReference {
    /// Casper mainnet (`casper:casper`).
    pub const CASPER: Self = Self::Mainnet;

    /// Casper testnet (`casper:casper-test`).
    pub const CASPER_TEST: Self = Self::Testnet;

    /// All chain references with built-in support.
    pub const ALL: &'static [Self] = &[Self::Mainnet, Self::Testnet];

    /// Returns the chain name (identical to the CAIP-2 reference).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mainnet => "casper",
            Self::Testnet => "casper-test",
        }
    }

    /// Returns the `chain_name` to embed into a Casper transaction.
    #[must_use]
    pub const fn chain_name(self) -> &'static str {
        self.as_str()
    }

    /// Returns the public JSON-RPC endpoint operated by the Casper
    /// Association for this network.
    #[must_use]
    pub const fn default_rpc_url(self) -> &'static str {
        match self {
            Self::Mainnet => "https://node.mainnet.casper.network/rpc",
            Self::Testnet => "https://node.testnet.casper.network/rpc",
        }
    }

    /// Returns `true` when this reference denotes a test network.
    #[must_use]
    pub const fn is_testnet(self) -> bool {
        matches!(self, Self::Testnet)
    }
}

impl Debug for CasperChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "CasperChainReference({})", self.as_str())
    }
}

impl Display for CasperChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CasperChainReference {
    type Err = CasperChainReferenceFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == s)
            .ok_or_else(|| CasperChainReferenceFormatError::InvalidReference(s.to_owned()))
    }
}

impl Serialize for CasperChainReference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CasperChainReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl From<CasperChainReference> for ChainId {
    fn from(value: CasperChainReference) -> Self {
        Self::new(CASPER_NAMESPACE, value.as_str())
    }
}

impl TryFrom<ChainId> for CasperChainReference {
    type Error = CasperChainReferenceFormatError;

    fn try_from(value: ChainId) -> Result<Self, Self::Error> {
        let (namespace, reference) = value.into_parts();
        if namespace != CASPER_NAMESPACE {
            return Err(CasperChainReferenceFormatError::InvalidNamespace(namespace));
        }
        Self::from_str(&reference)
    }
}

/// Error type for parsing Casper chain references.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CasperChainReferenceFormatError {
    /// The namespace was not `casper`.
    #[error("Invalid namespace {0}, expected casper")]
    InvalidNamespace(String),
    /// The reference did not name a known Casper network.
    #[error("Invalid casper chain reference {0}")]
    InvalidReference(String),
}

/// Errors produced while parsing a CEP-18 contract package hash.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ContractPackageHashParseError {
    /// The value was not exactly 64 hex characters.
    #[error("contract package hash must be 64 hex characters, got {0}")]
    InvalidLength(usize),
    /// The value contained non-hex characters.
    #[error("contract package hash is not valid hex: {0}")]
    NotHex(#[from] HexDecodeError),
}

/// A CEP-18 contract package hash — the x402 `asset` identifier on Casper.
///
/// Unlike [`super::Address`], a package hash is transported untagged.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContractPackageHash([u8; 32]);

impl ContractPackageHash {
    /// Builds a package hash from raw bytes.
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the raw 32-byte hash.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns `true` when `value` is a well-formed package hash.
    #[must_use]
    pub fn is_valid(value: &str) -> bool {
        Self::from_str(value).is_ok()
    }
}

impl Debug for ContractPackageHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "ContractPackageHash({self})")
    }
}

impl Display for ContractPackageHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&codec::encode(&self.0))
    }
}

impl FromStr for ContractPackageHash {
    type Err = ContractPackageHashParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != HASH_HEX_LEN {
            return Err(ContractPackageHashParseError::InvalidLength(s.len()));
        }
        Ok(Self(codec::decode_exact::<32>(s)?))
    }
}

impl Serialize for ContractPackageHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ContractPackageHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Information about a CEP-18 token deployment on a Casper network.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CasperTokenDeployment {
    /// The Casper network this token is deployed on.
    pub chain_reference: CasperChainReference,
    /// The CEP-18 contract package hash.
    pub address: ContractPackageHash,
    /// Number of decimal places (9 for wCSPR).
    pub decimals: u8,
    /// EIP-712 domain `name` used by the token's
    /// `transfer_with_authorization` entry point.
    pub name: &'static str,
    /// EIP-712 domain `version` used by the token's
    /// `transfer_with_authorization` entry point.
    pub version: &'static str,
}

impl CasperTokenDeployment {
    /// Creates a new token deployment descriptor.
    #[must_use]
    pub const fn new(
        chain_reference: CasperChainReference,
        address: ContractPackageHash,
        decimals: u8,
        name: &'static str,
        version: &'static str,
    ) -> Self {
        Self {
            chain_reference,
            address,
            decimals,
            name,
            version,
        }
    }

    /// Pairs a raw base-unit amount with this deployment.
    #[must_use]
    pub const fn amount(&self, motes: u128) -> DeployedTokenAmount<Motes, Self> {
        DeployedTokenAmount {
            amount: Motes::new(motes),
            token: *self,
        }
    }

    /// Parses a human-readable decimal amount into base units.
    ///
    /// # Errors
    ///
    /// Returns [`MotesParseError`] when the input is malformed or carries
    /// more precision than the token's `decimals` allow. Sub-unit precision
    /// is never truncated.
    pub fn parse(&self, value: &str) -> Result<DeployedTokenAmount<Motes, Self>, MotesParseError> {
        if self.decimals != CSPR_DECIMALS {
            return Err(MotesParseError::SubMotePrecision {
                digits: usize::from(self.decimals),
            });
        }
        Ok(DeployedTokenAmount {
            amount: Motes::from_cspr_str(value)?,
            token: *self,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACKAGE_HEX: &str = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";

    fn account() -> String {
        format!("00{PACKAGE_HEX}")
    }

    #[test]
    fn chain_reference_maps_to_caip2() {
        let chain_id: ChainId = CasperChainReference::CASPER.into();
        assert_eq!(chain_id.to_string(), "casper:casper");
        let testnet: ChainId = CasperChainReference::CASPER_TEST.into();
        assert_eq!(testnet.to_string(), "casper:casper-test");
    }

    #[test]
    fn chain_reference_round_trips_through_chain_id() {
        for reference in CasperChainReference::ALL {
            let chain_id: ChainId = (*reference).into();
            assert_eq!(
                CasperChainReference::try_from(chain_id).unwrap(),
                *reference
            );
        }
    }

    #[test]
    fn chain_reference_rejects_foreign_namespace() {
        let err = CasperChainReference::try_from(ChainId::new("eip155", "casper")).unwrap_err();
        assert!(matches!(
            err,
            CasperChainReferenceFormatError::InvalidNamespace(ref ns) if ns == "eip155"
        ));
    }

    #[test]
    fn chain_reference_rejects_unknown_network() {
        assert!("casper-dev".parse::<CasperChainReference>().is_err());
    }

    #[test]
    fn chain_reference_exposes_chain_name_and_rpc() {
        assert_eq!(CasperChainReference::CASPER.chain_name(), "casper");
        assert_eq!(
            CasperChainReference::CASPER_TEST.chain_name(),
            "casper-test"
        );
        assert!(CasperChainReference::CASPER_TEST.is_testnet());
        assert!(!CasperChainReference::CASPER.is_testnet());
        assert!(
            CasperChainReference::CASPER_TEST
                .default_rpc_url()
                .contains("testnet")
        );
    }

    #[test]
    fn contract_package_hash_is_untagged() {
        let hash: ContractPackageHash = PACKAGE_HEX.parse().unwrap();
        assert_eq!(hash.to_string(), PACKAGE_HEX);
        assert_eq!(hash.as_bytes().len(), 32);
        assert!(ContractPackageHash::is_valid(PACKAGE_HEX));
        assert!(!ContractPackageHash::is_valid(&account()));
    }

    #[test]
    fn deployment_parses_exact_amounts_only() {
        let deployment = CasperTokenDeployment::new(
            CasperChainReference::CASPER,
            PACKAGE_HEX.parse().unwrap(),
            CSPR_DECIMALS,
            "Wrapped CSPR",
            "1",
        );
        assert_eq!(
            deployment.parse("2.5").unwrap().amount.inner(),
            2_500_000_000
        );
        assert_eq!(deployment.amount(7).amount.inner(), 7);
        assert!(matches!(
            deployment.parse("0.0000000001").unwrap_err(),
            MotesParseError::SubMotePrecision { digits: 10 }
        ));
    }
}
