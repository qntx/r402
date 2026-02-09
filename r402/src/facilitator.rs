//! x402 facilitator base logic.
//!
//! Contains shared logic for facilitator implementations, including scheme
//! registration, routing, and supported-kinds aggregation.
//!
//! Corresponds to Python SDK's `facilitator_base.py`.

use std::collections::HashSet;

use crate::error::SchemeNotFoundError;
use crate::scheme::{SchemeFacilitator, SchemeFacilitatorV1};
use r402_proto::helpers::matches_network_pattern;
use r402_proto::{
    Network, PaymentPayload, PaymentPayloadV1, PaymentRequirements, PaymentRequirementsV1,
    SettleResponse, SupportedKind, SupportedResponse, VerifyResponse,
};

/// Internal storage for a registered scheme facilitator.
struct SchemeData<T: ?Sized> {
    facilitator: Box<T>,
    networks: HashSet<Network>,
    pattern: Network,
}

/// Derives a common CAIP pattern from a set of networks.
///
/// If all networks share the same namespace, returns a wildcard pattern.
fn derive_pattern(networks: &HashSet<Network>) -> Network {
    let namespaces: HashSet<&str> = networks
        .iter()
        .filter_map(|n| n.split(':').next())
        .collect();
    if namespaces.len() == 1 {
        let ns = namespaces.into_iter().next().expect("non-empty set");
        format!("{ns}:*")
    } else {
        networks.iter().next().cloned().unwrap_or_default()
    }
}

/// Base x402 facilitator with shared registration, routing, and supported logic.
///
/// Corresponds to Python SDK's `x402FacilitatorBase`.
pub struct X402FacilitatorBase {
    schemes_v2: Vec<SchemeData<dyn SchemeFacilitator>>,
    schemes_v1: Vec<SchemeData<dyn SchemeFacilitatorV1>>,
    extensions: Vec<String>,
}

impl std::fmt::Debug for X402FacilitatorBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X402FacilitatorBase")
            .field("schemes_v2_count", &self.schemes_v2.len())
            .field("schemes_v1_count", &self.schemes_v1.len())
            .field("extensions", &self.extensions)
            .finish()
    }
}

impl Default for X402FacilitatorBase {
    fn default() -> Self {
        Self::new()
    }
}

impl X402FacilitatorBase {
    /// Creates a new facilitator base.
    #[must_use]
    pub fn new() -> Self {
        Self {
            schemes_v2: Vec::new(),
            schemes_v1: Vec::new(),
            extensions: Vec::new(),
        }
    }

    /// Registers a V2 facilitator for one or more networks.
    pub fn register(
        &mut self,
        networks: Vec<Network>,
        facilitator: Box<dyn SchemeFacilitator>,
    ) -> &mut Self {
        let net_set: HashSet<Network> = networks.into_iter().collect();
        let pattern = derive_pattern(&net_set);
        self.schemes_v2.push(SchemeData {
            facilitator,
            networks: net_set,
            pattern,
        });
        self
    }

    /// Registers a V1 facilitator for one or more networks.
    pub fn register_v1(
        &mut self,
        networks: Vec<Network>,
        facilitator: Box<dyn SchemeFacilitatorV1>,
    ) -> &mut Self {
        let net_set: HashSet<Network> = networks.into_iter().collect();
        let pattern = derive_pattern(&net_set);
        self.schemes_v1.push(SchemeData {
            facilitator,
            networks: net_set,
            pattern,
        });
        self
    }

    /// Registers an extension name (e.g., `"bazaar"`).
    pub fn register_extension(&mut self, extension: String) -> &mut Self {
        if !self.extensions.contains(&extension) {
            self.extensions.push(extension);
        }
        self
    }

    /// Returns the list of registered extension names.
    #[must_use]
    pub fn extensions(&self) -> &[String] {
        &self.extensions
    }

    /// Aggregates supported payment kinds and signers from all registered
    /// scheme facilitators.
    ///
    /// Corresponds to Python SDK's `get_supported`.
    #[must_use]
    pub fn get_supported(&self) -> SupportedResponse {
        let mut kinds = Vec::new();
        let mut signers = std::collections::HashMap::<String, Vec<String>>::new();

        for data in &self.schemes_v2 {
            let fac = &data.facilitator;
            for network in &data.networks {
                kinds.push(SupportedKind {
                    x402_version: 2,
                    scheme: fac.scheme().to_owned(),
                    network: network.clone(),
                    extra: fac.get_extra(network),
                });

                let family = fac.caip_family().to_owned();
                let network_signers = fac.get_signers(network);
                let entry = signers.entry(family).or_default();
                for s in network_signers {
                    if !entry.contains(&s) {
                        entry.push(s);
                    }
                }
            }
        }

        for data in &self.schemes_v1 {
            let fac = &data.facilitator;
            for network in &data.networks {
                kinds.push(SupportedKind {
                    x402_version: 1,
                    scheme: fac.scheme().to_owned(),
                    network: network.clone(),
                    extra: fac.get_extra(network),
                });

                let family = fac.caip_family().to_owned();
                let network_signers = fac.get_signers(network);
                let entry = signers.entry(family).or_default();
                for s in network_signers {
                    if !entry.contains(&s) {
                        entry.push(s);
                    }
                }
            }
        }

        SupportedResponse::new(kinds, self.extensions.clone(), signers)
    }

    /// Finds the V2 facilitator for a given scheme and network.
    fn find_facilitator(&self, scheme: &str, network: &str) -> Option<&dyn SchemeFacilitator> {
        for data in &self.schemes_v2 {
            if data.facilitator.scheme() != scheme {
                continue;
            }
            if data.networks.contains(network) {
                return Some(&*data.facilitator);
            }
            if matches_network_pattern(network, &data.pattern) {
                return Some(&*data.facilitator);
            }
        }
        None
    }

    /// Finds the V1 facilitator for a given scheme and network.
    fn find_facilitator_v1(&self, scheme: &str, network: &str) -> Option<&dyn SchemeFacilitatorV1> {
        for data in &self.schemes_v1 {
            if data.facilitator.scheme() != scheme {
                continue;
            }
            if data.networks.contains(network) {
                return Some(&*data.facilitator);
            }
            if matches_network_pattern(network, &data.pattern) {
                return Some(&*data.facilitator);
            }
        }
        None
    }

    /// Verifies a V2 payment.
    ///
    /// # Errors
    ///
    /// Returns [`SchemeNotFoundError`] if no facilitator is registered for the
    /// payload's scheme and network.
    pub fn verify_v2(
        &self,
        payload: &PaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<VerifyResponse, SchemeNotFoundError> {
        let scheme = payload.scheme();
        let network = payload.network();
        let fac = self
            .find_facilitator(scheme, network)
            .ok_or_else(|| SchemeNotFoundError::new(scheme, network))?;
        Ok(fac.verify(payload, requirements))
    }

    /// Verifies a V1 payment.
    ///
    /// # Errors
    ///
    /// Returns [`SchemeNotFoundError`] if no facilitator is registered.
    pub fn verify_v1(
        &self,
        payload: &PaymentPayloadV1,
        requirements: &PaymentRequirementsV1,
    ) -> Result<VerifyResponse, SchemeNotFoundError> {
        let scheme = payload.scheme();
        let network = payload.network();
        let fac = self
            .find_facilitator_v1(scheme, network)
            .ok_or_else(|| SchemeNotFoundError::new(scheme, network))?;
        Ok(fac.verify(payload, requirements))
    }

    /// Settles a V2 payment.
    ///
    /// # Errors
    ///
    /// Returns [`SchemeNotFoundError`] if no facilitator is registered.
    pub fn settle_v2(
        &self,
        payload: &PaymentPayload,
        requirements: &PaymentRequirements,
    ) -> Result<SettleResponse, SchemeNotFoundError> {
        let scheme = payload.scheme();
        let network = payload.network();
        let fac = self
            .find_facilitator(scheme, network)
            .ok_or_else(|| SchemeNotFoundError::new(scheme, network))?;
        Ok(fac.settle(payload, requirements))
    }

    /// Settles a V1 payment.
    ///
    /// # Errors
    ///
    /// Returns [`SchemeNotFoundError`] if no facilitator is registered.
    pub fn settle_v1(
        &self,
        payload: &PaymentPayloadV1,
        requirements: &PaymentRequirementsV1,
    ) -> Result<SettleResponse, SchemeNotFoundError> {
        let scheme = payload.scheme();
        let network = payload.network();
        let fac = self
            .find_facilitator_v1(scheme, network)
            .ok_or_else(|| SchemeNotFoundError::new(scheme, network))?;
        Ok(fac.settle(payload, requirements))
    }
}
