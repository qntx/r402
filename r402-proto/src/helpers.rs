//! Utility functions for the x402 protocol.
//!
//! Provides version detection, payload parsing, and network pattern matching
//! utilities used across the protocol stack.

use serde_json::Value;

use crate::v1::{PaymentPayloadV1, PaymentRequiredV1, PaymentRequirementsV1};
use crate::v2::{PaymentPayload, PaymentRequired, PaymentRequirements};
use crate::{Network, ProtocolError};

/// Extracts the `x402Version` field from JSON data.
///
/// # Errors
///
/// Returns [`ProtocolError::MissingVersion`] if the field is absent.
/// Returns [`ProtocolError::InvalidVersion`] if the value is not 1 or 2.
pub fn detect_version(data: &Value) -> Result<u32, ProtocolError> {
    let version = data
        .get("x402Version")
        .ok_or(ProtocolError::MissingVersion)?;

    let version = version.as_u64().ok_or(ProtocolError::InvalidVersion(0))?;

    #[allow(clippy::cast_possible_truncation)]
    match version {
        1 | 2 => Ok(version as u32),
        _ => Err(ProtocolError::InvalidVersion(version as u32)),
    }
}

/// Extracts the `x402Version` from raw JSON bytes.
///
/// # Errors
///
/// Returns [`ProtocolError`] on parse failure or invalid version.
pub fn detect_version_bytes(data: &[u8]) -> Result<u32, ProtocolError> {
    let parsed: Value = serde_json::from_slice(data)?;
    detect_version(&parsed)
}

/// Extracts scheme and network from a payment payload.
///
/// - **V1**: `scheme` and `network` are at the top level.
/// - **V2**: `scheme` and `network` are inside the `accepted` field.
///
/// # Errors
///
/// Returns [`ProtocolError`] if required fields are missing.
pub fn get_scheme_and_network(
    version: u32,
    payload: &Value,
) -> Result<(String, String), ProtocolError> {
    let (scheme_val, network_val) = if version == 1 {
        (payload.get("scheme"), payload.get("network"))
    } else {
        let accepted = payload
            .get("accepted")
            .ok_or(ProtocolError::MissingField("accepted"))?;
        (accepted.get("scheme"), accepted.get("network"))
    };

    let scheme = scheme_val
        .and_then(Value::as_str)
        .ok_or(ProtocolError::MissingField("scheme"))?
        .to_owned();

    let network = network_val
        .and_then(Value::as_str)
        .ok_or(ProtocolError::MissingField("network"))?
        .to_owned();

    Ok((scheme, network))
}

/// Checks if a payment payload matches the given requirements.
///
/// - **V1**: Compares `scheme` and `network`.
/// - **V2**: Compares `scheme`, `network`, `amount`, `asset`, and `payTo`.
#[must_use]
pub fn match_payload_to_requirements(version: u32, payload: &Value, requirements: &Value) -> bool {
    if version == 1 {
        payload.get("scheme") == requirements.get("scheme")
            && payload.get("network") == requirements.get("network")
    } else {
        let Some(accepted) = payload.get("accepted") else {
            return false;
        };
        accepted.get("scheme") == requirements.get("scheme")
            && accepted.get("network") == requirements.get("network")
            && accepted.get("amount") == requirements.get("amount")
            && accepted.get("asset") == requirements.get("asset")
            && accepted.get("payTo") == requirements.get("payTo")
    }
}

/// Parses a 402 response into the appropriate version type.
///
/// Auto-detects version from the `x402Version` field.
///
/// # Errors
///
/// Returns [`ProtocolError`] on parse failure.
pub fn parse_payment_required(data: &Value) -> Result<PaymentRequiredEnum, ProtocolError> {
    let version = detect_version(data)?;
    if version == 1 {
        let v1: PaymentRequiredV1 = serde_json::from_value(data.clone())?;
        Ok(PaymentRequiredEnum::V1(Box::new(v1)))
    } else {
        let v2: PaymentRequired = serde_json::from_value(data.clone())?;
        Ok(PaymentRequiredEnum::V2(Box::new(v2)))
    }
}

/// Parses a 402 response from raw JSON bytes.
///
/// # Errors
///
/// Returns [`ProtocolError`] on parse failure.
pub fn parse_payment_required_bytes(data: &[u8]) -> Result<PaymentRequiredEnum, ProtocolError> {
    let parsed: Value = serde_json::from_slice(data)?;
    parse_payment_required(&parsed)
}

/// Parses a payment payload into the appropriate version type.
///
/// Auto-detects version from the `x402Version` field.
///
/// # Errors
///
/// Returns [`ProtocolError`] on parse failure.
pub fn parse_payment_payload(data: &Value) -> Result<PaymentPayloadEnum, ProtocolError> {
    let version = detect_version(data)?;
    if version == 1 {
        let v1: PaymentPayloadV1 = serde_json::from_value(data.clone())?;
        Ok(PaymentPayloadEnum::V1(Box::new(v1)))
    } else {
        let v2: PaymentPayload = serde_json::from_value(data.clone())?;
        Ok(PaymentPayloadEnum::V2(Box::new(v2)))
    }
}

/// Parses a payment payload from raw JSON bytes.
///
/// # Errors
///
/// Returns [`ProtocolError`] on parse failure.
pub fn parse_payment_payload_bytes(data: &[u8]) -> Result<PaymentPayloadEnum, ProtocolError> {
    let parsed: Value = serde_json::from_slice(data)?;
    parse_payment_payload(&parsed)
}

/// Parses payment requirements based on the protocol version.
///
/// Unlike [`parse_payment_payload`], requirements don't contain
/// `x402Version` — the version must be provided from the corresponding
/// payment payload.
///
/// # Errors
///
/// Returns [`ProtocolError`] on parse failure or invalid version.
pub fn parse_payment_requirements(
    x402_version: u32,
    data: &Value,
) -> Result<PaymentRequirementsEnum, ProtocolError> {
    match x402_version {
        1 => {
            let v1: PaymentRequirementsV1 = serde_json::from_value(data.clone())?;
            Ok(PaymentRequirementsEnum::V1(Box::new(v1)))
        }
        2 => {
            let v2: PaymentRequirements = serde_json::from_value(data.clone())?;
            Ok(PaymentRequirementsEnum::V2(Box::new(v2)))
        }
        _ => Err(ProtocolError::InvalidVersion(x402_version)),
    }
}

/// Checks if a network matches a pattern (supports wildcards).
///
/// Patterns ending with `:*` match any reference within the namespace.
#[must_use]
pub fn matches_network_pattern(network: &str, pattern: &str) -> bool {
    pattern
        .strip_suffix('*')
        .map_or_else(|| pattern == network, |prefix| network.starts_with(prefix))
}

/// Derives a common CAIP pattern from a list of networks.
///
/// If all networks share the same namespace (e.g., "eip155"), returns a
/// wildcard pattern (e.g., "eip155:*"). Otherwise returns the first network.
///
/// # Panics
///
/// Panics if the networks slice is empty.
#[must_use]
pub fn derive_network_pattern(networks: &[&str]) -> String {
    assert!(!networks.is_empty(), "at least one network required");

    let namespaces: std::collections::HashSet<&str> = networks
        .iter()
        .filter_map(|n| n.split(':').next())
        .collect();

    if namespaces.len() == 1 {
        let ns = namespaces.into_iter().next().expect("non-empty set");
        format!("{ns}:*")
    } else {
        networks[0].to_owned()
    }
}

/// Finds schemes registered for a network (with wildcard matching).
///
/// Tries exact match first, then falls back to wildcard patterns.
#[must_use]
pub fn find_schemes_by_network<'a, T, S: std::hash::BuildHasher>(
    schemes: &'a std::collections::HashMap<Network, T, S>,
    network: &str,
) -> Option<&'a T> {
    if let Some(v) = schemes.get(network) {
        return Some(v);
    }

    for (pattern, scheme_map) in schemes {
        if matches_network_pattern(network, pattern) {
            return Some(scheme_map);
        }
    }

    None
}

/// A version-tagged `PaymentRequired` enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaymentRequiredEnum {
    /// V1 format.
    V1(Box<PaymentRequiredV1>),
    /// V2 format.
    V2(Box<PaymentRequired>),
}

/// A version-tagged `PaymentPayload` enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaymentPayloadEnum {
    /// V1 format.
    V1(Box<PaymentPayloadV1>),
    /// V2 format.
    V2(Box<PaymentPayload>),
}

/// A version-tagged `PaymentRequirements` enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaymentRequirementsEnum {
    /// V1 format.
    V1(Box<PaymentRequirementsV1>),
    /// V2 format.
    V2(Box<PaymentRequirements>),
}

impl PaymentPayloadEnum {
    /// Returns the protocol version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        match self {
            Self::V1(p) => p.x402_version,
            Self::V2(p) => p.x402_version,
        }
    }

    /// Returns the payment scheme.
    #[must_use]
    pub fn scheme(&self) -> &str {
        match self {
            Self::V1(p) => p.scheme(),
            Self::V2(p) => p.scheme(),
        }
    }

    /// Returns the network.
    #[must_use]
    pub fn network(&self) -> &str {
        match self {
            Self::V1(p) => p.network(),
            Self::V2(p) => p.network(),
        }
    }
}

impl PaymentRequirementsEnum {
    /// Returns the payment scheme.
    #[must_use]
    pub fn scheme(&self) -> &str {
        match self {
            Self::V1(r) => &r.scheme,
            Self::V2(r) => &r.scheme,
        }
    }

    /// Returns the network.
    #[must_use]
    pub fn network(&self) -> &str {
        match self {
            Self::V1(r) => &r.network,
            Self::V2(r) => &r.network,
        }
    }

    /// Returns the payment amount.
    #[must_use]
    pub fn amount(&self) -> &str {
        match self {
            Self::V1(r) => r.amount(),
            Self::V2(r) => r.amount(),
        }
    }
}
