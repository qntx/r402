//! 402 body construction: scheme enrich, extension advertise, payment-flow extra.

use compact_str::CompactString;
use r402_protocol::error::FacilitatorError;
use r402_protocol::extension::{AdvertiseContext, ExtensionRegistry};
use r402_protocol::network::ChainId;
use r402_protocol::payment::{
    Extensions, PaymentRequired, PaymentRequirements, ResourceInfo, SupportedResponse,
};

use crate::hooks::{
    WirePaymentPayload, assert_accepts_additive_extra_after_scheme_enrich,
    assert_accepts_allowlisted_after_extension_enrich, snapshot_payment_requirements_list,
};
use crate::payment_flow::apply_payment_flow_wire_extra;
use crate::resource::ResourceServer;
use crate::scheme::SchemePaymentRequiredContext;

/// Inputs for [`ResourceServer::create_payment_required_response`].
#[derive(Debug, Clone)]
pub struct PaymentRequiredBuildContext {
    /// Resource metadata placed on the 402 body.
    pub resource: ResourceInfo,
    /// Optional error string (`PaymentRequired.error`).
    pub error: Option<CompactString>,
    /// Declared extension entries copied onto the 402 body before advertise.
    pub extensions: Extensions,
    /// Facilitator `/supported` snapshot (scheme enrich reads this later).
    pub supported: SupportedResponse,
    /// Failed payment payload, when building a 402 after a paid attempt.
    pub payment_payload: Option<WirePaymentPayload>,
}

impl ResourceServer {
    /// One 402 body used for both unpaid challenges and paid matching.
    ///
    /// After scheme enrich and extension advertise, each accept is resolved
    /// through the registered scheme table and written with
    /// [`apply_payment_flow_wire_extra`]. Unregistered schemes fail closed.
    ///
    /// # Errors
    ///
    /// [`FacilitatorError::Internal`] when hook policy, unregistered scheme,
    /// or payment-flow resolution / wire extra application fails.
    pub async fn create_payment_required_response(
        &self,
        accepts: Vec<PaymentRequirements>,
        ctx: PaymentRequiredBuildContext,
    ) -> Result<PaymentRequired, FacilitatorError> {
        let PaymentRequiredBuildContext {
            resource,
            error,
            extensions,
            supported,
            payment_payload,
        } = ctx;
        let mut response = PaymentRequired::new(resource)
            .with_accepts(accepts)
            .with_extensions(extensions);
        if let Some(error) = error {
            response = response.with_error(error);
        }
        self.apply_scheme_payment_required_enrich(
            &mut response,
            &supported,
            payment_payload.as_ref(),
        )
        .await?;
        advertise_registered_extensions(&self.extensions, &mut response)?;
        self.apply_payment_flow_extras(&mut response.accepts)?;
        Ok(response)
    }

    /// Scheme-table resolve, then [`apply_payment_flow_wire_extra`].
    fn apply_payment_flow_extras(
        &self,
        accepts: &mut [PaymentRequirements],
    ) -> Result<(), FacilitatorError> {
        for accept in accepts {
            let resolved = self
                .resolved_payment_flow(accept)
                .map_err(FacilitatorError::internal)?;
            let next = apply_payment_flow_wire_extra(accept.extra.as_ref(), &resolved);
            accept.extra = if next.is_empty() {
                None
            } else {
                Some(serde_json::Value::Object(next))
            };
        }
        Ok(())
    }

    /// Per-accept scheme enrich, then additive-extra hook policy.
    async fn apply_scheme_payment_required_enrich(
        &self,
        response: &mut PaymentRequired,
        supported: &SupportedResponse,
        payment_payload: Option<&WirePaymentPayload>,
    ) -> Result<(), FacilitatorError> {
        let targets: Vec<(CompactString, ChainId)> = response
            .accepts
            .iter()
            .map(|accept| (accept.scheme.clone(), accept.network.clone()))
            .collect();
        let mut baseline = snapshot_payment_requirements_list(&response.accepts);
        for (scheme_name, network) in targets {
            let Some(scheme) = self.registered_scheme(scheme_name.as_str(), &network) else {
                continue;
            };
            let network_label = network.to_string();
            let enriched = {
                let ctx = SchemePaymentRequiredContext {
                    requirements: &response.accepts,
                    payment_payload,
                    resource: &response.resource,
                    error: response.error.as_deref(),
                    payment_required_response: response,
                    supported,
                };
                scheme.enrich_payment_required_response(&ctx).await
            };
            if let Some(accepts) = enriched {
                response.accepts = accepts;
            }
            assert_accepts_additive_extra_after_scheme_enrich(
                &baseline,
                &response.accepts,
                scheme_name.as_str(),
                network_label.as_str(),
            )
            .map_err(FacilitatorError::internal)?;
            baseline = snapshot_payment_requirements_list(&response.accepts);
        }
        Ok(())
    }
}

fn advertise_registered_extensions(
    registry: &ExtensionRegistry,
    response: &mut PaymentRequired,
) -> Result<(), FacilitatorError> {
    for ext in registry.iter() {
        let baseline = snapshot_payment_requirements_list(&response.accepts);
        if response.extensions.get(ext.id()).is_none()
            && let Some(entry) = ext.advertise(&AdvertiseContext::new(None))
        {
            response.extensions.insert(ext.id(), entry);
        }
        assert_accepts_allowlisted_after_extension_enrich(&baseline, &response.accepts, ext.id())
            .map_err(FacilitatorError::internal)?;
    }
    Ok(())
}
