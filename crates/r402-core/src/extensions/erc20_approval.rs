//! Server advertise helper for `erc20ApprovalGasSponsoring` (official wire).
//!
//! Client signed-tx payload types live in `r402-evm`. This module only
//! declares the extension on `PaymentRequired` for resource servers.

#![cfg(feature = "ext-erc20-approval")]
#![cfg_attr(docsrs, doc(cfg(feature = "ext-erc20-approval")))]

use serde_json::{Value, json};

use super::{AdvertiseContext, Extension};
use crate::wire::ExtensionEntry;

/// Stable extension key on the wire.
pub const ERC20_APPROVAL_GAS_SPONSORING_KEY: &str = "erc20ApprovalGasSponsoring";

/// Current schema version for server advertise info.
pub const ERC20_APPROVAL_GAS_SPONSORING_VERSION: &str = "1";

/// Official client `info` JSON Schema (from `erc20_gas_sponsoring.md`).
fn client_info_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "from": { "type": "string", "pattern": "^0x[a-fA-F0-9]{40}$" },
            "asset": { "type": "string", "pattern": "^0x[a-fA-F0-9]{40}$" },
            "spender": { "type": "string", "pattern": "^0x[a-fA-F0-9]{40}$" },
            "amount": { "type": "string", "pattern": "^[0-9]+$" },
            "signedTransaction": { "type": "string", "pattern": "^0x[a-fA-F0-9]+$" },
            "version": { "type": "string", "pattern": "^[0-9]+(\\.[0-9]+)*$" }
        },
        "required": ["from", "asset", "spender", "amount", "signedTransaction", "version"]
    })
}

/// Resource-server declaration of ERC-20 approval gas sponsoring support.
#[derive(Debug, Clone)]
pub struct Erc20ApprovalGasSponsoringExtension {
    /// Human-readable description in advertise `info`.
    pub description: String,
    /// Schema version string.
    pub version: String,
}

impl Default for Erc20ApprovalGasSponsoringExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl Erc20ApprovalGasSponsoringExtension {
    /// Official default description and version `"1"`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            description:
                "The facilitator accepts a raw signed approval transaction and will sponsor the gas fees."
                    .into(),
            version: ERC20_APPROVAL_GAS_SPONSORING_VERSION.into(),
        }
    }
}

impl Extension for Erc20ApprovalGasSponsoringExtension {
    fn id(&self) -> &'static str {
        ERC20_APPROVAL_GAS_SPONSORING_KEY
    }

    fn advertise(&self, _ctx: &AdvertiseContext<'_>) -> Option<ExtensionEntry> {
        Some(ExtensionEntry::with_schema(
            json!({
                "description": self.description,
                "version": self.version,
            }),
            client_info_schema(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertise_key_and_version() {
        let ext = Erc20ApprovalGasSponsoringExtension::new();
        assert_eq!(ext.id(), "erc20ApprovalGasSponsoring");
        let entry = ext
            .advertise(&AdvertiseContext { requirement: None })
            .unwrap();
        let v = entry.to_value();
        assert_eq!(v["info"]["version"], "1");
    }
}
