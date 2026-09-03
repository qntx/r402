//! Server advertise helper for `eip2612GasSponsoring` (official wire).
//!
//! Payload signing/parsing lives in `r402-evm`. This module only declares
//! the extension on `PaymentRequired` for resource servers.

use r402_protocol::extension::{AdvertiseContext, Extension};
use r402_protocol::payment::ExtensionEntry;
use serde_json::{Value, json};

/// Stable extension key on the wire.
pub const EIP2612_GAS_SPONSORING_KEY: &str = "eip2612GasSponsoring";

/// Current schema version for server advertise info.
pub const EIP2612_GAS_SPONSORING_VERSION: &str = "1";

/// Official client `info` JSON Schema (from `eip2612_gas_sponsoring.md`).
fn client_info_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "from": { "type": "string", "pattern": "^0x[a-fA-F0-9]{40}$" },
            "asset": { "type": "string", "pattern": "^0x[a-fA-F0-9]{40}$" },
            "spender": { "type": "string", "pattern": "^0x[a-fA-F0-9]{40}$" },
            "amount": { "type": "string", "pattern": "^[0-9]+$" },
            "nonce": { "type": "string", "pattern": "^[0-9]+$" },
            "deadline": { "type": "string", "pattern": "^[0-9]+$" },
            "signature": { "type": "string", "pattern": "^0x[a-fA-F0-9]+$" },
            "version": { "type": "string", "pattern": "^[0-9]+(\\.[0-9]+)*$" }
        },
        "required": ["from", "asset", "spender", "amount", "nonce", "deadline", "signature", "version"]
    })
}

/// Resource-server declaration of EIP-2612 gas sponsoring support.
#[derive(Debug, Clone)]
pub struct Eip2612GasSponsoringExtension {
    /// Human-readable description in advertise `info`.
    pub description: String,
    /// Schema version string.
    pub version: String,
}

impl Default for Eip2612GasSponsoringExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl Eip2612GasSponsoringExtension {
    /// Official default description and version `"1"`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            description:
                "The facilitator accepts EIP-2612 gasless Permit to `Permit2` canonical contract."
                    .into(),
            version: EIP2612_GAS_SPONSORING_VERSION.into(),
        }
    }
}

impl Extension for Eip2612GasSponsoringExtension {
    fn id(&self) -> &'static str {
        EIP2612_GAS_SPONSORING_KEY
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
        let ext = Eip2612GasSponsoringExtension::new();
        assert_eq!(ext.id(), "eip2612GasSponsoring");
        let Some(entry) = ext.advertise(&AdvertiseContext::new(None)) else {
            panic!("eip2612 advertise must emit an entry");
        };
        let v = entry.to_value();
        assert_eq!(v["info"]["version"], "1");
        assert!(
            v.get("schema").is_some(),
            "client schema must be advertised"
        );
    }
}
