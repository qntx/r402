//! x402 client base logic.
//!
//! Contains shared logic for client implementations, including scheme
//! registration, requirement selection policies, and payment creation.
//!
//! Corresponds to Python SDK's `client_base.py`.

use std::collections::HashMap;

use r402_proto::helpers::find_schemes_by_network;
use r402_proto::{
    Network, PaymentPayload, PaymentPayloadV1, PaymentRequired, PaymentRequiredV1,
    PaymentRequirements, PaymentRequirementsV1,
};

use crate::error::{NoMatchingRequirementsError, SchemeNotFoundError};
use crate::scheme::{SchemeClient, SchemeClientV1};

/// Policy function that filters and reorders requirements.
///
/// Takes the protocol version and a list of requirements, returns a
/// filtered/reordered list. Corresponds to Python SDK's `PaymentPolicy`.
pub type PaymentPolicy =
    Box<dyn Fn(u32, Vec<RequirementsView>) -> Vec<RequirementsView> + Send + Sync>;

/// Selector function that picks the final requirement from a filtered list.
///
/// Corresponds to Python SDK's `PaymentRequirementsSelector`.
pub type PaymentRequirementsSelector = Box<dyn Fn(u32, &[RequirementsView]) -> usize + Send + Sync>;

/// A version-agnostic view of payment requirements for use in policies.
#[derive(Debug, Clone)]
pub enum RequirementsView {
    /// V2 requirements.
    V2(PaymentRequirements),
    /// V1 requirements.
    V1(PaymentRequirementsV1),
}

impl RequirementsView {
    /// Returns the scheme identifier.
    #[must_use]
    pub fn scheme(&self) -> &str {
        match self {
            Self::V2(r) => &r.scheme,
            Self::V1(r) => &r.scheme,
        }
    }

    /// Returns the network identifier.
    #[must_use]
    pub fn network(&self) -> &str {
        match self {
            Self::V2(r) => &r.network,
            Self::V1(r) => &r.network,
        }
    }

    /// Returns the payment amount as a string.
    #[must_use]
    pub fn amount(&self) -> &str {
        match self {
            Self::V2(r) => r.amount(),
            Self::V1(r) => r.amount(),
        }
    }
}

/// Creates a policy that prefers a specific network.
///
/// Requirements matching the given network are placed first.
#[must_use]
pub fn prefer_network(network: Network) -> PaymentPolicy {
    Box::new(move |_version, reqs| {
        let mut preferred = Vec::new();
        let mut others = Vec::new();
        for r in reqs {
            if r.network() == network {
                preferred.push(r);
            } else {
                others.push(r);
            }
        }
        preferred.extend(others);
        preferred
    })
}

/// Creates a policy that prefers a specific scheme.
///
/// Requirements matching the given scheme are placed first.
#[must_use]
pub fn prefer_scheme(scheme: String) -> PaymentPolicy {
    Box::new(move |_version, reqs| {
        let mut preferred = Vec::new();
        let mut others = Vec::new();
        for r in reqs {
            if r.scheme() == scheme {
                preferred.push(r);
            } else {
                others.push(r);
            }
        }
        preferred.extend(others);
        preferred
    })
}

/// Creates a policy that filters by maximum amount.
///
/// Only requirements with `amount <= max_value` are kept.
#[must_use]
pub fn max_amount(max_value: u128) -> PaymentPolicy {
    Box::new(move |_version, reqs| {
        reqs.into_iter()
            .filter(|r| r.amount().parse::<u128>().is_ok_and(|a| a <= max_value))
            .collect()
    })
}

/// Default selector: returns the first requirement.
const fn default_selector(_version: u32, _reqs: &[RequirementsView]) -> usize {
    0
}

/// Base x402 client with shared registration, policy, and selection logic.
///
/// Corresponds to Python SDK's `x402ClientBase`.
pub struct X402ClientBase {
    schemes_v2: HashMap<Network, HashMap<String, Box<dyn SchemeClient>>>,
    schemes_v1: HashMap<Network, HashMap<String, Box<dyn SchemeClientV1>>>,
    policies: Vec<PaymentPolicy>,
    selector: PaymentRequirementsSelector,
}

impl std::fmt::Debug for X402ClientBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X402ClientBase")
            .field(
                "schemes_v2_networks",
                &self.schemes_v2.keys().collect::<Vec<_>>(),
            )
            .field(
                "schemes_v1_networks",
                &self.schemes_v1.keys().collect::<Vec<_>>(),
            )
            .field("policies_count", &self.policies.len())
            .finish_non_exhaustive()
    }
}

impl Default for X402ClientBase {
    fn default() -> Self {
        Self::new()
    }
}

impl X402ClientBase {
    /// Creates a new client base with default selector.
    #[must_use]
    pub fn new() -> Self {
        Self {
            schemes_v2: HashMap::new(),
            schemes_v1: HashMap::new(),
            policies: Vec::new(),
            selector: Box::new(default_selector),
        }
    }

    /// Creates a new client base with a custom selector.
    #[must_use]
    pub fn with_selector(selector: PaymentRequirementsSelector) -> Self {
        Self {
            schemes_v2: HashMap::new(),
            schemes_v1: HashMap::new(),
            policies: Vec::new(),
            selector,
        }
    }

    /// Registers a V2 scheme client for a network.
    pub fn register(&mut self, network: Network, client: Box<dyn SchemeClient>) -> &mut Self {
        let scheme = client.scheme().to_owned();
        self.schemes_v2
            .entry(network)
            .or_default()
            .insert(scheme, client);
        self
    }

    /// Registers a V1 scheme client for a network.
    pub fn register_v1(&mut self, network: Network, client: Box<dyn SchemeClientV1>) -> &mut Self {
        let scheme = client.scheme().to_owned();
        self.schemes_v1
            .entry(network)
            .or_default()
            .insert(scheme, client);
        self
    }

    /// Adds a requirement filter policy.
    pub fn register_policy(&mut self, policy: PaymentPolicy) -> &mut Self {
        self.policies.push(policy);
        self
    }

    /// Selects V2 requirements using policies and selector.
    ///
    /// # Errors
    ///
    /// Returns [`NoMatchingRequirementsError`] if no requirements match.
    pub fn select_requirements_v2(
        &self,
        requirements: &[PaymentRequirements],
    ) -> Result<PaymentRequirements, NoMatchingRequirementsError> {
        let supported: Vec<RequirementsView> = requirements
            .iter()
            .filter(|req| {
                find_schemes_by_network(&self.schemes_v2, &req.network)
                    .is_some_and(|schemes| schemes.contains_key(&req.scheme))
            })
            .cloned()
            .map(RequirementsView::V2)
            .collect();

        if supported.is_empty() {
            return Err(NoMatchingRequirementsError::new(
                "No payment requirements match registered schemes",
            ));
        }

        let mut filtered = supported;
        for policy in &self.policies {
            filtered = policy(2, filtered);
            if filtered.is_empty() {
                return Err(NoMatchingRequirementsError::new(
                    "All requirements filtered out by policies",
                ));
            }
        }

        let idx = (self.selector)(2, &filtered);
        match filtered.into_iter().nth(idx) {
            Some(RequirementsView::V2(r)) => Ok(r),
            _ => Err(NoMatchingRequirementsError::new(
                "Selector returned invalid index",
            )),
        }
    }

    /// Selects V1 requirements using policies and selector.
    ///
    /// # Errors
    ///
    /// Returns [`NoMatchingRequirementsError`] if no requirements match.
    pub fn select_requirements_v1(
        &self,
        requirements: &[PaymentRequirementsV1],
    ) -> Result<PaymentRequirementsV1, NoMatchingRequirementsError> {
        let supported: Vec<RequirementsView> = requirements
            .iter()
            .filter(|req| {
                find_schemes_by_network(&self.schemes_v1, &req.network)
                    .is_some_and(|schemes| schemes.contains_key(&req.scheme))
            })
            .cloned()
            .map(RequirementsView::V1)
            .collect();

        if supported.is_empty() {
            return Err(NoMatchingRequirementsError::new(
                "No payment requirements match registered schemes",
            ));
        }

        let mut filtered = supported;
        for policy in &self.policies {
            filtered = policy(1, filtered);
            if filtered.is_empty() {
                return Err(NoMatchingRequirementsError::new(
                    "All requirements filtered out by policies",
                ));
            }
        }

        let idx = (self.selector)(1, &filtered);
        match filtered.into_iter().nth(idx) {
            Some(RequirementsView::V1(r)) => Ok(r),
            _ => Err(NoMatchingRequirementsError::new(
                "Selector returned invalid index",
            )),
        }
    }

    /// Creates a V2 payment payload from a 402 response.
    ///
    /// # Errors
    ///
    /// Returns an error if requirement selection or payload creation fails.
    pub fn create_payment_payload_v2(
        &self,
        payment_required: &PaymentRequired,
    ) -> Result<PaymentPayload, Box<dyn std::error::Error + Send + Sync>> {
        let selected = self.select_requirements_v2(&payment_required.accepts)?;

        let schemes = find_schemes_by_network(&self.schemes_v2, &selected.network)
            .ok_or_else(|| SchemeNotFoundError::new(&selected.scheme, &selected.network))?;

        let client = schemes
            .get(&selected.scheme)
            .ok_or_else(|| SchemeNotFoundError::new(&selected.scheme, &selected.network))?;

        let inner_payload = client.create_payment_payload(&selected)?;

        Ok(PaymentPayload {
            x402_version: 2,
            payload: inner_payload,
            resource: payment_required.resource.clone(),
            extensions: payment_required.extensions.clone(),
            accepted: selected,
        })
    }

    /// Creates a V1 payment payload from a V1 402 response.
    ///
    /// # Errors
    ///
    /// Returns an error if requirement selection or payload creation fails.
    pub fn create_payment_payload_v1(
        &self,
        payment_required: &PaymentRequiredV1,
    ) -> Result<PaymentPayloadV1, Box<dyn std::error::Error + Send + Sync>> {
        let selected = self.select_requirements_v1(&payment_required.accepts)?;

        let schemes = find_schemes_by_network(&self.schemes_v1, &selected.network)
            .ok_or_else(|| SchemeNotFoundError::new(&selected.scheme, &selected.network))?;

        let client = schemes
            .get(&selected.scheme)
            .ok_or_else(|| SchemeNotFoundError::new(&selected.scheme, &selected.network))?;

        let inner_payload = client.create_payment_payload(&selected)?;

        Ok(PaymentPayloadV1 {
            x402_version: 1,
            scheme: selected.scheme,
            network: selected.network,
            payload: inner_payload,
        })
    }
}
