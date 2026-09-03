//! Payment envelopes for the x402 protocol.
//!
//! JSON is camelCase with `deny_unknown_fields`. Identifier strings use
//! [`compact_str::CompactString`].

mod codec;
mod extensions;
mod overrides;
mod payload;
mod price;
mod required;
mod requirements;
mod resource;
mod settle;
mod supported;
mod verify;

use std::str::FromStr;

pub use codec::{Base64Bytes, U64String, UnixTimestamp, V2, Version, Version2};
pub use extensions::{EXTENSION_RESPONSE_LOG_FIELDS, ExtensionEntry, Extensions};
pub use overrides::{
    DEFAULT_ASSET_DECIMALS, SettlementOverrideError, SettlementOverrides,
    asset_decimals_from_extra, resolve_settlement_override_amount,
};
pub use payload::PaymentPayload;
pub use price::PriceTag;
pub use required::PaymentRequired;
pub use requirements::{PaymentRequirements, find_matching_requirements};
pub use resource::ResourceInfo;
pub use settle::{SettleRequest, SettleResponse};
pub use supported::{SupportedPaymentKind, SupportedResponse};
pub use verify::{TypedVerifyRequest, VerifyRequest, VerifyResponse};

use crate::network::ChainId;
use crate::scheme::SchemeSlug;

pub(crate) fn scheme_slug_from_json(json: &serde_json::Value) -> Option<SchemeSlug> {
    let version = json.get("x402Version")?.as_u64()?;
    let version: u8 = version.try_into().ok()?;
    if version != Version2::VALUE {
        return None;
    }
    let accepted = json.get("paymentPayload")?.get("accepted")?;
    let chain_id = ChainId::from_str(accepted.get("network")?.as_str()?).ok()?;
    let scheme = accepted.get("scheme")?.as_str()?;
    Some(SchemeSlug::new(chain_id, scheme.into()))
}

pub(crate) fn network_from_json(json: &serde_json::Value) -> &str {
    json.get("paymentRequirements")
        .and_then(|req| req.get("network"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}
