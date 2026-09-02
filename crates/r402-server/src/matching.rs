//! Requirement matching against a client payload.

use r402_protocol::payment::PaymentRequirements;

use crate::hooks::WirePaymentPayload;
use crate::resource::ResourceServer;
use crate::scheme::DynSchemeNetworkServer;

impl ResourceServer {
    /// First available accept that matches `payload.accepted`.
    ///
    /// Scheme-declared [`crate::SchemeNetworkServer::dynamic_extra_fields`] are
    /// omitted from the extra-subset comparison.
    #[must_use]
    pub fn find_matching_requirements<'a>(
        &self,
        available: &'a [PaymentRequirements],
        payload: &WirePaymentPayload,
    ) -> Option<&'a PaymentRequirements> {
        available.iter().find(|req| {
            let dynamic = self
                .registered_scheme(req.scheme.as_str(), &req.network)
                .map_or(&[][..], DynSchemeNetworkServer::dynamic_extra_fields);
            req.matches_payload_accepted_with_dynamic(&payload.accepted, dynamic)
        })
    }
}
