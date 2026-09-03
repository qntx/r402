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

pub mod echo;
pub mod error;
pub mod extension;
pub mod metrics;
pub mod money;
pub mod network;
pub mod payment;
pub mod scheme;

pub use echo::validate_extension_echoes;
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
    resolve_settlement_override_amount, schema_has_external_ref,
};
#[cfg(test)]
use proptest as _;
pub use scheme::{
    AuthCaptureScheme, BatchSettlementScheme, ExactScheme, SchemeId, SchemeMarkerError, SchemeSlug,
    UptoScheme,
};
