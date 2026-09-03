//! ERC-20 approval gas-sponsoring extension wire types (exact + upto).
//!
//! Official key: `erc20ApprovalGasSponsoring`. Clients attach a signed
//! `approve(Permit2, max)` transaction; facilitators broadcast it before
//! Permit2 `settle` when the extension is registered.

use alloy_primitives::Address;
use r402_protocol::payment::{ExtensionEntry, Extensions};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable extension identifier on the wire.
pub const ERC20_APPROVAL_GAS_SPONSORING_KEY: &str = "erc20ApprovalGasSponsoring";

/// Schema version written into client-populated `info.version`.
pub const ERC20_APPROVAL_GAS_SPONSORING_VERSION: &str = "1";

/// Official gas limit for the sponsored `approve` transaction.
pub const ERC20_APPROVE_GAS_LIMIT: u64 = 70_000;

/// Fallback `maxFeePerGas` (1 gwei) when fee estimation is unavailable.
pub const DEFAULT_MAX_FEE_PER_GAS: u128 = 1_000_000_000;

/// Fallback `maxPriorityFeePerGas` (0.1 gwei) when fee estimation is unavailable.
pub const DEFAULT_MAX_PRIORITY_FEE_PER_GAS: u128 = 100_000_000;

/// Buyer-signed ERC-20 `approve` transaction for gasless Permit2 allowance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Erc20ApprovalGasSponsoringInfo {
    /// Token owner (== payment payer).
    pub from: Address,
    /// ERC-20 token contract.
    pub asset: Address,
    /// Spender — MUST be the canonical Permit2 address.
    pub spender: Address,
    /// Approved allowance (uint256 decimal string). Official always uses `maxUint256`.
    pub amount: String,
    /// RLP-encoded signed EIP-1559 `approve` transaction (`0x`-prefixed hex).
    pub signed_transaction: String,
    /// Schema version (`"1"`).
    pub version: String,
}

/// Errors raised while extracting [`Erc20ApprovalGasSponsoringInfo`] from extensions.
#[derive(Debug, Error)]
pub enum Erc20ApprovalParseError {
    /// Extension JSON did not match the expected shape.
    #[error("invalid erc20ApprovalGasSponsoring extension payload: {0}")]
    Invalid(#[from] serde_json::Error),
}

impl Erc20ApprovalGasSponsoringInfo {
    /// Decodes the extension from a payload extensions map.
    ///
    /// # Errors
    ///
    /// Malformed JSON.
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
        Ok(Some(parsed))
    }

    /// Builds a structured extension entry for payload insertion.
    ///
    /// # Errors
    ///
    /// Serde failures (should not occur for this type).
    pub fn to_extension_entry(&self) -> Result<ExtensionEntry, serde_json::Error> {
        Ok(ExtensionEntry::info(serde_json::to_value(self)?))
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;
    use r402_protocol::payment::Extensions;

    use super::*;

    fn sample() -> Erc20ApprovalGasSponsoringInfo {
        Erc20ApprovalGasSponsoringInfo {
            from: Address::repeat_byte(0xAA),
            asset: Address::repeat_byte(0xBB),
            spender: Address::repeat_byte(0xCC),
            amount:
                "115792089237316195423570985008687907853269984665640564039457584007913129639935"
                    .into(),
            signed_transaction: "0xdeadbeef".into(),
            version: ERC20_APPROVAL_GAS_SPONSORING_VERSION.into(),
        }
    }

    #[test]
    fn wire_uses_signed_transaction_camel_case() {
        let v = serde_json::to_value(sample()).unwrap();
        assert!(v.get("signedTransaction").is_some());
        assert!(v.get("signed_transaction").is_none());
        assert_eq!(v.get("version"), Some(&serde_json::json!("1")));
    }

    #[test]
    fn extension_round_trips_structured() {
        let info = sample();
        let mut extensions = Extensions::new();
        extensions.insert(
            ERC20_APPROVAL_GAS_SPONSORING_KEY,
            info.to_extension_entry().unwrap(),
        );
        let extracted = Erc20ApprovalGasSponsoringInfo::from_extensions(&extensions)
            .unwrap()
            .unwrap();
        assert_eq!(extracted, info);
    }

    #[test]
    fn missing_extension_ok_none() {
        assert!(
            Erc20ApprovalGasSponsoringInfo::from_extensions(&Extensions::new())
                .unwrap()
                .is_none()
        );
    }
}
