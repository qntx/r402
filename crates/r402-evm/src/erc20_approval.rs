//! `erc20ApprovalGasSponsoring` extension wire types.
//!
//! Official key and client `info` shape from
//! `specs/extensions/erc20_gas_sponsoring.md`. Facilitators that sponsor gas
//! for non-EIP-2612 tokens broadcast `signedTransaction` then settle.

use alloy_primitives::{Address, Bytes};
use r402_core::wire::{ExtensionEntry, Extensions};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::chain::TokenAmount;

/// Stable extension identifier on the wire.
pub const ERC20_APPROVAL_GAS_SPONSORING_KEY: &str = "erc20ApprovalGasSponsoring";

/// Schema version for client-populated `info.version`.
pub const ERC20_APPROVAL_GAS_SPONSORING_VERSION: &str = "1";

/// Client-signed ERC-20 `approve` transaction for gas-sponsored Permit2 setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Erc20ApprovalGasSponsoringInfo {
    /// Token owner / transaction sender.
    pub from: Address,
    /// ERC-20 token to approve.
    pub asset: Address,
    /// Spender (canonical Permit2).
    pub spender: Address,
    /// Approval amount (typically `MaxUint256`).
    pub amount: TokenAmount,
    /// RLP-encoded signed `approve` transaction (`0x…`).
    pub signed_transaction: Bytes,
    /// Schema version (`"1"`).
    pub version: String,
}

/// Errors extracting the extension from a payload.
#[derive(Debug, Error)]
pub enum Erc20ApprovalParseError {
    /// Malformed extension JSON.
    #[error("invalid erc20ApprovalGasSponsoring extension payload: {0}")]
    Invalid(#[from] serde_json::Error),
    /// Signed transaction payload empty.
    #[error("erc20ApprovalGasSponsoring.signedTransaction is empty")]
    EmptyTransaction,
}

impl Erc20ApprovalGasSponsoringInfo {
    /// Decodes the extension from payload extensions.
    ///
    /// # Errors
    ///
    /// Malformed JSON or empty transaction bytes.
    pub fn from_extensions(
        extensions: &Extensions,
    ) -> Result<Option<Self>, Erc20ApprovalParseError> {
        let Some(entry) = extensions.get(ERC20_APPROVAL_GAS_SPONSORING_KEY) else {
            return Ok(None);
        };
        let parsed: Self = match entry {
            ExtensionEntry::Structured { info, .. } => serde_json::from_value(info.clone())?,
            ExtensionEntry::Raw(value) => serde_json::from_value(value.clone())?,
        };
        if parsed.signed_transaction.is_empty() {
            return Err(Erc20ApprovalParseError::EmptyTransaction);
        }
        Ok(Some(parsed))
    }

    /// Builds a structured extension entry.
    ///
    /// # Errors
    ///
    /// Serde failures.
    pub fn to_extension_entry(&self) -> Result<ExtensionEntry, serde_json::Error> {
        Ok(ExtensionEntry::info(serde_json::to_value(self)?))
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, Bytes, U256};
    use r402_core::wire::Extensions;

    use super::*;

    fn sample() -> Erc20ApprovalGasSponsoringInfo {
        Erc20ApprovalGasSponsoringInfo {
            from: Address::repeat_byte(0x11),
            asset: Address::repeat_byte(0x22),
            spender: Address::repeat_byte(0x33),
            amount: TokenAmount::from(U256::MAX),
            signed_transaction: Bytes::from(vec![0x02, 0xf8, 0x01]),
            version: ERC20_APPROVAL_GAS_SPONSORING_VERSION.into(),
        }
    }

    #[test]
    fn wire_field_names() {
        let v = serde_json::to_value(sample()).unwrap();
        assert!(v.get("signedTransaction").is_some());
        assert_eq!(v.get("version"), Some(&serde_json::json!("1")));
    }

    #[test]
    fn round_trip() {
        let info = sample();
        let mut ext = Extensions::new();
        ext.insert(
            ERC20_APPROVAL_GAS_SPONSORING_KEY,
            info.to_extension_entry().unwrap(),
        );
        let out = Erc20ApprovalGasSponsoringInfo::from_extensions(&ext)
            .unwrap()
            .unwrap();
        assert_eq!(out, info);
    }
}
