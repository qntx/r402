//! Shared EVM transfer-method and validator constants.

use alloy_primitives::{Address, address};
use serde::{Deserialize, Serialize};

/// Universal signature verifier for EIP-6492, EIP-1271, and EOA signatures.
///
/// Deployed at the same address on every supported EVM chain. The exact and
/// upto facilitators delegate to this contract via
/// `Validator6492::isValidSigWithSideEffects`; if the validator is missing on
/// a particular chain, EIP-6492 / EIP-1271 verification will revert and only
/// EOA signatures will work. Use the `validator_deployed_on_configured_chains`
/// integration test to confirm presence on every chain you operate.
///
/// This address is **r402-specific** (not currently part of the cross-SDK
/// canonical-address set). Out-of-tree deployments must mirror the bytecode
/// to maintain interop.
pub const VALIDATOR_ADDRESS: Address = address!("0xdAcD51A54883eb67D95FAEb2BBfdC4a9a6BD2a3B");

/// Determines which on-chain mechanism is used for token transfers.
///
/// Supported by r402-evm:
/// - `eip3009` ([`AssetTransferMethod::Eip3009`]): `transferWithAuthorization`
/// - `permit2` ([`AssetTransferMethod::Permit2`]): Permit2 + `x402Permit2Proxy`
///
/// Spec also defines **ERC-7710** (`scheme_exact_evm.md` section 3). That path
/// is **not implemented** here (and is absent from the official Go/TS mechanism
/// packages as of the vendored foundation tree). Deserialising
/// `"assetTransferMethod": "erc7710"` (or any other unknown value) fails with
/// an explicit unsupported-method error rather than a silent untagged-payload
/// mis-parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AssetTransferMethod {
    /// EIP-3009 `transferWithAuthorization`.
    Eip3009,
    /// Uniswap Permit2 via `x402Permit2Proxy`.
    Permit2,
}

impl<'de> Deserialize<'de> for AssetTransferMethod {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        match raw.as_str() {
            "eip3009" => Ok(Self::Eip3009),
            "permit2" => Ok(Self::Permit2),
            other => Err(serde::de::Error::custom(format!(
                "unsupported assetTransferMethod `{other}`: r402-evm implements \
                 eip3009 and permit2 only (ERC-7710 is not implemented)"
            ))),
        }
    }
}

#[cfg(test)]
mod transfer_method_tests {
    use super::AssetTransferMethod;

    #[test]
    fn serialises_as_camel_case_wire_strings() {
        assert_eq!(
            serde_json::to_string(&AssetTransferMethod::Eip3009).unwrap(),
            "\"eip3009\""
        );
        assert_eq!(
            serde_json::to_string(&AssetTransferMethod::Permit2).unwrap(),
            "\"permit2\""
        );
    }

    #[test]
    fn deserialises_supported_methods() {
        assert_eq!(
            serde_json::from_str::<AssetTransferMethod>("\"eip3009\"").unwrap(),
            AssetTransferMethod::Eip3009
        );
        assert_eq!(
            serde_json::from_str::<AssetTransferMethod>("\"permit2\"").unwrap(),
            AssetTransferMethod::Permit2
        );
    }

    #[test]
    fn rejects_erc7710_with_explicit_message() {
        let err = serde_json::from_str::<AssetTransferMethod>("\"erc7710\"").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unsupported assetTransferMethod `erc7710`"),
            "unexpected error: {msg}"
        );
        assert!(
            msg.contains("ERC-7710 is not implemented"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn rejects_unknown_method() {
        let err = serde_json::from_str::<AssetTransferMethod>("\"somethingElse\"").unwrap_err();
        assert!(err.to_string().contains("unsupported assetTransferMethod"));
    }
}
