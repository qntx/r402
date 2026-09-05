//! Per-scheme resource-server adapter.

use std::collections::HashMap;
use std::future::Future;

use compact_str::CompactString;
use r402_facilitator::BoxFuture;
use r402_protocol::error::FacilitatorError;
use r402_protocol::network::ChainId;
use r402_protocol::payment::{
    PaymentRequired, PaymentRequirements, ResourceInfo, SupportedPaymentKind, SupportedResponse,
};
use serde_json::{Map, Value};

use crate::hooks::{
    SettleContext, SettleResultContext, VerifiedPaymentCanceledContext, WirePaymentPayload,
};
use crate::payment_flow::PaymentFlowConfig;

/// Context for scheme 402 enrichment.
#[derive(Debug)]
#[non_exhaustive]
pub struct SchemePaymentRequiredContext<'a> {
    /// Accepts being enriched.
    pub requirements: &'a [PaymentRequirements],
    /// Client payload when enriching a paid 402 retry.
    pub payment_payload: Option<&'a WirePaymentPayload>,
    /// Resource metadata from the 402.
    pub resource: &'a ResourceInfo,
    /// Optional 402 error string.
    pub error: Option<&'a str>,
    /// Working payment-required response.
    pub payment_required_response: &'a PaymentRequired,
    /// Facilitator `GET /supported` snapshot.
    pub supported: &'a SupportedResponse,
    /// CAIP-2 of the accept this enrich invocation is bound to.
    pub network: &'a ChainId,
}

impl<'a> SchemePaymentRequiredContext<'a> {
    /// Constructs a 402 enrich context.
    #[must_use]
    pub const fn new(
        requirements: &'a [PaymentRequirements],
        resource: &'a ResourceInfo,
        payment_required_response: &'a PaymentRequired,
        supported: &'a SupportedResponse,
        network: &'a ChainId,
    ) -> Self {
        Self {
            requirements,
            payment_payload: None,
            resource,
            error: None,
            payment_required_response,
            supported,
            network,
        }
    }
}

/// Scheme/network adapter registered on [`crate::ResourceServer`].
///
/// Enrich hooks are AFIT async. Defaults are no-ops.
pub trait SchemeNetworkServer: Send + Sync {
    /// Wire scheme name (e.g. `"exact"`).
    fn scheme(&self) -> &str;

    /// ATM used when `requirements.extra.assetTransferMethod` is absent.
    fn default_asset_transfer_method(&self) -> &str;

    /// Payment flows supported per asset-transfer method.
    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig>;

    /// Extra keys omitted from requirement matching.
    fn dynamic_extra_fields(&self) -> &[&str] {
        &[]
    }

    /// Optional 402 accept enrichment. `None` leaves accepts unchanged.
    fn enrich_payment_required_response<'a>(
        &'a self,
        _ctx: &'a SchemePaymentRequiredContext<'a>,
    ) -> impl Future<Output = Option<Vec<PaymentRequirements>>> + Send + 'a {
        async { None }
    }

    /// Optional additive settle-payload enrichment.
    ///
    /// `Ok(None)` leaves the client payload unchanged. `Ok(Some(map))` is
    /// merged additively. `Err` fails the settle.
    fn enrich_settlement_payload<'a>(
        &'a self,
        _ctx: &'a SettleContext,
    ) -> impl Future<Output = Result<Option<Map<String, Value>>, FacilitatorError>> + Send + 'a
    {
        async { Ok(None) }
    }

    /// Optional additive settle-response enrichment.
    fn enrich_settlement_response<'a>(
        &'a self,
        _ctx: &'a SettleResultContext,
    ) -> impl Future<Output = Option<Map<String, Value>>> + Send + 'a {
        async { None }
    }

    /// Requirements to settle when a verified payment is canceled.
    ///
    /// `None` skips cancel settle.
    fn settle_on_cancel<'a>(
        &'a self,
        _ctx: &'a VerifiedPaymentCanceledContext,
    ) -> impl Future<Output = Option<PaymentRequirements>> + Send + 'a {
        async { None }
    }

    /// `true` iff [`Self::settle_on_cancel`] can return `Some`.
    ///
    /// HTTP construct cannot await [`Self::settle_on_cancel`]. Default `false`.
    fn settles_on_cancel(&self) -> bool {
        false
    }

    /// Advertised `/supported` kind extra for this scheme. Default `Ok(())`.
    ///
    /// # Errors
    ///
    /// [`FacilitatorSupportError`] when the advertised kind cannot serve this scheme.
    fn validate_facilitator_support(
        &self,
        network: &ChainId,
        kind: &SupportedPaymentKind,
        facilitator_extensions: &[CompactString],
    ) -> Result<(), FacilitatorSupportError> {
        let _ = (network, kind, facilitator_extensions);
        Ok(())
    }
}

/// Facilitator `/supported` kind is missing or its extra is unusable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FacilitatorSupportError {
    /// No matching kind for this accept.
    #[error("{scheme} on {network}: facilitator does not advertise this kind")]
    KindMissing {
        /// Wire scheme name.
        scheme: CompactString,
        /// Accept network.
        network: ChainId,
    },
    /// Advertised extra omits `feePayer`.
    #[error("{scheme} on {network}: missing extra.feePayer")]
    MissingFeePayer {
        /// Wire scheme name.
        scheme: CompactString,
        /// Accept network.
        network: ChainId,
    },
    /// Advertised `feePayer` is not a valid address.
    #[error("{scheme} on {network}: invalid extra.feePayer")]
    InvalidFeePayer {
        /// Wire scheme name.
        scheme: CompactString,
        /// Accept network.
        network: ChainId,
    },
    /// Advertised extra omits `receiverAuthorizer`.
    #[error("{scheme} on {network}: missing extra.receiverAuthorizer")]
    MissingReceiverAuthorizer {
        /// Wire scheme name.
        scheme: CompactString,
        /// Accept network.
        network: ChainId,
    },
    /// Advertised `receiverAuthorizer` is the zero address.
    #[error("{scheme} on {network}: extra.receiverAuthorizer is the zero address")]
    ZeroReceiverAuthorizer {
        /// Wire scheme name.
        scheme: CompactString,
        /// Accept network.
        network: ChainId,
    },
    /// Advertised `receiverAuthorizer` is not a usable address.
    #[error("{scheme} on {network}: invalid extra.receiverAuthorizer")]
    InvalidReceiverAuthorizer {
        /// Wire scheme name.
        scheme: CompactString,
        /// Accept network.
        network: ChainId,
    },
}

impl FacilitatorSupportError {
    /// Stable machine reason for HTTP/MCP mapping.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::KindMissing { .. } => "kind_missing",
            Self::MissingFeePayer { .. } => "missing_fee_payer",
            Self::InvalidFeePayer { .. } => "invalid_fee_payer",
            Self::MissingReceiverAuthorizer { .. } => "missing_receiver_authorizer",
            Self::ZeroReceiverAuthorizer { .. } => "zero_receiver_authorizer",
            Self::InvalidReceiverAuthorizer { .. } => "invalid_receiver_authorizer",
        }
    }
}

/// Object-safe erasure of [`SchemeNetworkServer`].
pub trait DynSchemeNetworkServer: Send + Sync {
    /// See [`SchemeNetworkServer::scheme`].
    fn scheme(&self) -> &str;

    /// See [`SchemeNetworkServer::default_asset_transfer_method`].
    fn default_asset_transfer_method(&self) -> &str;

    /// See [`SchemeNetworkServer::payment_flows`].
    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig>;

    /// See [`SchemeNetworkServer::dynamic_extra_fields`].
    fn dynamic_extra_fields(&self) -> &[&str];

    /// See [`SchemeNetworkServer::enrich_payment_required_response`].
    fn enrich_payment_required_response<'a>(
        &'a self,
        ctx: &'a SchemePaymentRequiredContext<'a>,
    ) -> BoxFuture<'a, Option<Vec<PaymentRequirements>>>;

    /// See [`SchemeNetworkServer::enrich_settlement_payload`].
    fn enrich_settlement_payload<'a>(
        &'a self,
        ctx: &'a SettleContext,
    ) -> BoxFuture<'a, Result<Option<Map<String, Value>>, FacilitatorError>>;

    /// See [`SchemeNetworkServer::enrich_settlement_response`].
    fn enrich_settlement_response<'a>(
        &'a self,
        ctx: &'a SettleResultContext,
    ) -> BoxFuture<'a, Option<Map<String, Value>>>;

    /// See [`SchemeNetworkServer::settle_on_cancel`].
    fn settle_on_cancel<'a>(
        &'a self,
        ctx: &'a VerifiedPaymentCanceledContext,
    ) -> BoxFuture<'a, Option<PaymentRequirements>>;

    /// See [`SchemeNetworkServer::settles_on_cancel`].
    fn settles_on_cancel(&self) -> bool;

    /// See [`SchemeNetworkServer::validate_facilitator_support`].
    ///
    /// # Errors
    ///
    /// [`FacilitatorSupportError`] when the advertised kind cannot serve this scheme.
    fn validate_facilitator_support(
        &self,
        network: &ChainId,
        kind: &SupportedPaymentKind,
        facilitator_extensions: &[CompactString],
    ) -> Result<(), FacilitatorSupportError>;
}

impl<T: SchemeNetworkServer + ?Sized> DynSchemeNetworkServer for T {
    fn scheme(&self) -> &str {
        <Self as SchemeNetworkServer>::scheme(self)
    }

    fn default_asset_transfer_method(&self) -> &str {
        <Self as SchemeNetworkServer>::default_asset_transfer_method(self)
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        <Self as SchemeNetworkServer>::payment_flows(self)
    }

    fn dynamic_extra_fields(&self) -> &[&str] {
        <Self as SchemeNetworkServer>::dynamic_extra_fields(self)
    }

    fn enrich_payment_required_response<'a>(
        &'a self,
        ctx: &'a SchemePaymentRequiredContext<'a>,
    ) -> BoxFuture<'a, Option<Vec<PaymentRequirements>>> {
        Box::pin(<Self as SchemeNetworkServer>::enrich_payment_required_response(self, ctx))
    }

    fn enrich_settlement_payload<'a>(
        &'a self,
        ctx: &'a SettleContext,
    ) -> BoxFuture<'a, Result<Option<Map<String, Value>>, FacilitatorError>> {
        Box::pin(<Self as SchemeNetworkServer>::enrich_settlement_payload(
            self, ctx,
        ))
    }

    fn enrich_settlement_response<'a>(
        &'a self,
        ctx: &'a SettleResultContext,
    ) -> BoxFuture<'a, Option<Map<String, Value>>> {
        Box::pin(<Self as SchemeNetworkServer>::enrich_settlement_response(
            self, ctx,
        ))
    }

    fn settle_on_cancel<'a>(
        &'a self,
        ctx: &'a VerifiedPaymentCanceledContext,
    ) -> BoxFuture<'a, Option<PaymentRequirements>> {
        Box::pin(<Self as SchemeNetworkServer>::settle_on_cancel(self, ctx))
    }

    fn settles_on_cancel(&self) -> bool {
        <Self as SchemeNetworkServer>::settles_on_cancel(self)
    }

    fn validate_facilitator_support(
        &self,
        network: &ChainId,
        kind: &SupportedPaymentKind,
        facilitator_extensions: &[CompactString],
    ) -> Result<(), FacilitatorSupportError> {
        <Self as SchemeNetworkServer>::validate_facilitator_support(
            self,
            network,
            kind,
            facilitator_extensions,
        )
    }
}
