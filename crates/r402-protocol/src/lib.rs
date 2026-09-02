//! Wire types, CAIP-2 network identifiers, scheme markers, and money for
//! the x402 payment protocol (version 2).

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    test,
    allow(
        clippy::indexing_slicing,
        clippy::panic,
        reason = "unit tests panic on assertion failure"
    )
)]

pub mod error;
pub mod extension;
pub mod metrics;
pub mod money;
pub mod network;
pub mod payment;
pub mod scheme;

pub use error::{
    AsPaymentProblem, ClientError, ErrorReason, FacilitatorError, FacilitatorTransportKind,
    PaymentProblem, SettlementError, VerificationError,
};
pub use extension::{
    AdvertiseContext, BoxFuture, DynExtension, Extension, ExtensionRegistry, SettleContext,
    VerifyContext,
};
pub use money::{MoneyAmount, MoneyAmountParseError, ScaleFromMantissa};
pub use network::{
    ChainId, ChainIdFormatError, ChainIdPattern, ChainProvider, DeployedTokenAmount, NetworkInfo,
};
pub use payment::{
    Base64Bytes, DEFAULT_ASSET_DECIMALS, EXTENSION_RESPONSE_LOG_FIELDS, ExtensionEntry, Extensions,
    PaymentPayload, PaymentRequired, PaymentRequirements, PriceTag, ResourceInfo, SettleRequest,
    SettleResponse, SettlementOverrideError, SettlementOverrides, SupportedPaymentKind,
    SupportedResponse, TypedVerifyRequest, U64String, UnixTimestamp, V2, VerifyRequest,
    VerifyResponse, Version, Version2, asset_decimals_from_extra, find_matching_requirements,
    resolve_settlement_override_amount,
};
#[cfg(test)]
use proptest as _;
pub use scheme::{
    AuthCaptureScheme, BatchSettlementScheme, ExactScheme, SchemeId, SchemeMarkerError, SchemeSlug,
    UptoScheme,
};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-protocol",
            "package name must match the crate directory"
        );
    }
}
