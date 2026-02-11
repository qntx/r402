//! Payment scheme system for x402.
//!
//! This module provides the extensible scheme system that allows different
//! payment methods to be plugged into the x402 protocol. Each scheme defines
//! how payments are authorized, verified, and settled.
//!
//! # Facilitator-Side
//!
//! - [`X402SchemeFacilitator`] - Processes verify/settle requests
//! - [`X402SchemeBlueprint`] / [`SchemeBlueprints`] - Factories that create handlers
//! - [`SchemeRegistry`] - Maps chain+scheme combinations to handlers
//!
//! # Client-Side
//!
//! - [`X402SchemeClient`] - Generates [`PaymentCandidate`]s from 402 responses
//! - [`PaymentSelector`] - Chooses the best candidate ([`FirstMatch`], [`PreferChain`], [`MaxAmount`])

use alloy_primitives::U256;

use crate::chain::{ChainId, ChainIdPattern, ChainProviderOps};
use crate::proto;
use crate::proto::{AsPaymentProblem, ErrorReason, PaymentProblem, PaymentVerificationError};

use std::collections::HashMap;
use std::fmt;
use std::fmt::{Debug, Display, Formatter};
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;

/// Trait for scheme handlers that process payment verification and settlement.
///
/// Implementations of this trait handle the core payment processing logic:
/// verifying that payments are valid and settling them on-chain.
pub trait X402SchemeFacilitator: Send + Sync {
    /// Verifies a payment authorization without settling it.
    ///
    /// This checks that the payment is properly signed, matches the requirements,
    /// and the payer has sufficient funds.
    fn verify(
        &self,
        request: proto::VerifyRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<proto::VerifyResponse, X402SchemeFacilitatorError>>
                + Send
                + '_,
        >,
    >;

    /// Settles a verified payment on-chain.
    ///
    /// This submits the payment transaction to the blockchain and waits
    /// for confirmation.
    fn settle(
        &self,
        request: proto::SettleRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<proto::SettleResponse, X402SchemeFacilitatorError>>
                + Send
                + '_,
        >,
    >;

    /// Returns the payment methods supported by this handler.
    fn supported(
        &self,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<proto::SupportedResponse, X402SchemeFacilitatorError>>
                + Send
                + '_,
        >,
    >;
}

/// Marker trait for types that are both identifiable and buildable.
///
/// This combines [`X402SchemeId`] and [`X402SchemeFacilitatorBuilder`] for
/// use in the blueprint registry.
pub trait X402SchemeBlueprint<P>:
    X402SchemeId + for<'a> X402SchemeFacilitatorBuilder<&'a P>
{
}
impl<T, P> X402SchemeBlueprint<P> for T where
    T: X402SchemeId + for<'a> X402SchemeFacilitatorBuilder<&'a P>
{
}

/// Trait for identifying a payment scheme.
///
/// Each scheme has a unique identifier composed of the protocol version,
/// chain namespace, and scheme name.
pub trait X402SchemeId {
    /// Returns the x402 protocol version (1 or 2).
    fn x402_version(&self) -> u8 {
        2
    }
    /// Returns the chain namespace (e.g., "eip155", "solana").
    fn namespace(&self) -> &str;
    /// Returns the scheme name (e.g., "exact").
    fn scheme(&self) -> &str;
    /// Returns the full scheme identifier (e.g., "v2-eip155-exact").
    fn id(&self) -> String {
        format!(
            "v{}-{}-{}",
            self.x402_version(),
            self.namespace(),
            self.scheme(),
        )
    }
}

/// Trait for building scheme handlers from chain providers.
///
/// The type parameter `P` represents the chain provider type.
pub trait X402SchemeFacilitatorBuilder<P> {
    /// Creates a new scheme handler for the given chain provider.
    ///
    /// # Errors
    ///
    /// Returns an error if the handler cannot be built from the provider.
    fn build(
        &self,
        provider: P,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn X402SchemeFacilitator>, Box<dyn std::error::Error>>;
}

/// Errors that can occur during scheme operations.
#[derive(Debug, thiserror::Error)]
pub enum X402SchemeFacilitatorError {
    /// Payment verification failed.
    #[error(transparent)]
    PaymentVerification(#[from] PaymentVerificationError),
    /// On-chain operation failed.
    #[error("Onchain error: {0}")]
    OnchainFailure(String),
}

impl AsPaymentProblem for X402SchemeFacilitatorError {
    fn as_payment_problem(&self) -> PaymentProblem {
        match self {
            Self::PaymentVerification(e) => e.as_payment_problem(),
            Self::OnchainFailure(e) => PaymentProblem::new(ErrorReason::UnexpectedError, e.clone()),
        }
    }
}

/// Registry of scheme blueprints (factories).
///
/// Register blueprints at startup, then use them to build handlers
/// via [`SchemeRegistry`].
///
/// # Type Parameters
///
/// - `P` - The chain provider type
#[derive(Default)]
pub struct SchemeBlueprints<P>(
    HashMap<String, Box<dyn X402SchemeBlueprint<P>>>,
    PhantomData<P>,
);

impl<P> Debug for SchemeBlueprints<P> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let slugs: Vec<String> = self.0.keys().cloned().collect();
        f.debug_tuple("SchemeBlueprints").field(&slugs).finish()
    }
}

impl<P> SchemeBlueprints<P> {
    /// Creates an empty blueprint registry.
    #[must_use]
    pub fn new() -> Self {
        Self(HashMap::new(), PhantomData)
    }

    /// Registers a blueprint and returns self for chaining.
    #[must_use]
    pub fn and_register<B: X402SchemeBlueprint<P> + 'static>(mut self, blueprint: B) -> Self {
        self.register(blueprint);
        self
    }

    /// Registers a scheme blueprint.
    pub fn register<B: X402SchemeBlueprint<P> + 'static>(&mut self, blueprint: B) {
        self.0.insert(blueprint.id(), Box::new(blueprint));
    }

    /// Gets a blueprint by its ID.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&dyn X402SchemeBlueprint<P>> {
        self.0.get(id).map(|v| &**v)
    }
}

/// Unique identifier for a scheme handler instance.
///
/// Combines the chain ID, protocol version, and scheme name to uniquely
/// identify a handler that can process payments for a specific combination.
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct SchemeHandlerSlug {
    /// The chain this handler operates on.
    pub chain_id: ChainId,
    /// The x402 protocol version.
    pub x402_version: u8,
    /// The scheme name (e.g., "exact").
    pub name: String,
}

impl SchemeHandlerSlug {
    /// Creates a new scheme handler slug.
    #[must_use]
    pub const fn new(chain_id: ChainId, x402_version: u8, name: String) -> Self {
        Self {
            chain_id,
            x402_version,
            name,
        }
    }
}

impl Display for SchemeHandlerSlug {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:v{}:{}",
            self.chain_id.namespace, self.chain_id.reference, self.x402_version, self.name
        )
    }
}

/// Registry of active scheme handlers.
///
/// Maps chain+scheme combinations to their handlers.
#[derive(Default)]
pub struct SchemeRegistry(HashMap<SchemeHandlerSlug, Box<dyn X402SchemeFacilitator>>);

impl Debug for SchemeRegistry {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let slugs: Vec<String> = self.0.keys().map(ToString::to_string).collect();
        f.debug_tuple("SchemeRegistry").field(&slugs).finish()
    }
}

impl SchemeRegistry {
    /// Creates an empty scheme registry.
    #[must_use]
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    /// Registers a handler for a given blueprint and chain provider.
    ///
    /// # Errors
    ///
    /// Returns an error if the handler cannot be built from the provider.
    pub fn register<P: ChainProviderOps>(
        &mut self,
        blueprint: &dyn X402SchemeBlueprint<P>,
        provider: &P,
        config: Option<serde_json::Value>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let chain_id = provider.chain_id();
        let handler = blueprint.build(provider, config)?;
        let slug = SchemeHandlerSlug::new(
            chain_id,
            blueprint.x402_version(),
            blueprint.scheme().to_string(),
        );
        self.0.insert(slug, handler);
        Ok(())
    }

    /// Gets a handler by its slug.
    #[must_use]
    pub fn by_slug(&self, slug: &SchemeHandlerSlug) -> Option<&dyn X402SchemeFacilitator> {
        let handler = &**self.0.get(slug)?;
        Some(handler)
    }

    /// Returns an iterator over all registered handlers.
    pub fn values(&self) -> impl Iterator<Item = &dyn X402SchemeFacilitator> {
        self.0.values().map(|v| &**v)
    }
}

/// A payment option that can be signed and submitted.
///
/// Payment candidates are generated by scheme clients when they find
/// a matching payment requirement they can fulfill.
pub struct PaymentCandidate {
    /// The chain where payment will be made.
    pub chain_id: ChainId,
    /// The token asset address.
    pub asset: String,
    /// The payment amount in token units.
    pub amount: U256,
    /// The payment scheme name.
    pub scheme: String,
    /// The x402 protocol version.
    pub x402_version: u8,
    /// The recipient address.
    pub pay_to: String,
    /// The signer that can authorize this payment.
    pub signer: Box<dyn PaymentCandidateSigner + Send + Sync>,
}

impl Debug for PaymentCandidate {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaymentCandidate")
            .field("chain_id", &self.chain_id)
            .field("asset", &self.asset)
            .field("amount", &self.amount)
            .field("scheme", &self.scheme)
            .field("x402_version", &self.x402_version)
            .field("pay_to", &self.pay_to)
            .field("signer", &"<dyn PaymentCandidateSigner>")
            .finish()
    }
}

impl PaymentCandidate {
    /// Signs this payment candidate, producing a payment payload.
    ///
    /// # Errors
    ///
    /// Returns [`X402Error`] if signing fails.
    pub async fn sign(&self) -> Result<String, X402Error> {
        self.signer.sign_payment().await
    }
}

/// Trait for scheme clients that can process payment requirements.
///
/// Implementations examine 402 responses and generate payment candidates
/// for requirements they can fulfill.
pub trait X402SchemeClient: X402SchemeId + Send + Sync {
    /// Generates payment candidates for the given payment requirements.
    fn accept(&self, payment_required: &proto::PaymentRequired) -> Vec<PaymentCandidate>;
}

/// Trait for signing payment authorizations.
pub trait PaymentCandidateSigner {
    /// Signs a payment authorization.
    fn sign_payment(&self) -> Pin<Box<dyn Future<Output = Result<String, X402Error>> + Send + '_>>;
}

/// Errors that can occur during client-side payment processing.
#[derive(Debug, thiserror::Error)]
pub enum X402Error {
    /// No payment option matched the client's capabilities.
    #[error("No matching payment option found")]
    NoMatchingPaymentOption,

    /// The HTTP request body cannot be cloned (e.g., streaming).
    #[error("Request is not cloneable (streaming body?)")]
    RequestNotCloneable,

    /// Failed to parse the 402 response body.
    #[error("Failed to parse 402 response: {0}")]
    ParseError(String),

    /// Failed to sign the payment authorization.
    #[error("Failed to sign payment: {0}")]
    SigningError(String),

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
}

/// Trait for selecting the best payment candidate from available options.
///
/// Implement this trait to customize how payments are selected when
/// multiple options are available.
pub trait PaymentSelector: Send + Sync {
    /// Selects a payment candidate from the available options.
    fn select<'a>(&self, candidates: &'a [PaymentCandidate]) -> Option<&'a PaymentCandidate>;
}

/// Selector that returns the first matching candidate.
///
/// This is the simplest selection strategy. The order of candidates
/// is determined by the registration order of scheme clients.
#[derive(Debug, Clone, Copy)]
pub struct FirstMatch;

impl PaymentSelector for FirstMatch {
    fn select<'a>(&self, candidates: &'a [PaymentCandidate]) -> Option<&'a PaymentCandidate> {
        candidates.first()
    }
}

/// Selector that prefers specific chains in priority order.
///
/// Patterns are tried in order; the first matching candidate is returned.
/// If no patterns match, falls back to the first available candidate.
#[derive(Debug)]
pub struct PreferChain(Vec<ChainIdPattern>);

impl PreferChain {
    /// Creates a new chain preference selector.
    pub fn new<P: Into<Vec<ChainIdPattern>>>(patterns: P) -> Self {
        Self(patterns.into())
    }

    /// Adds additional chain patterns with lower priority.
    #[must_use]
    pub fn or_chain<P: Into<Vec<ChainIdPattern>>>(self, patterns: P) -> Self {
        Self(self.0.into_iter().chain(patterns.into()).collect())
    }
}

impl PaymentSelector for PreferChain {
    fn select<'a>(&self, candidates: &'a [PaymentCandidate]) -> Option<&'a PaymentCandidate> {
        for pattern in &self.0 {
            if let Some(candidate) = candidates.iter().find(|c| pattern.matches(&c.chain_id)) {
                return Some(candidate);
            }
        }
        candidates.first()
    }
}

/// Selector that only accepts payments up to a maximum amount.
///
/// Useful for limiting spending or implementing budget controls.
#[derive(Debug, Clone, Copy)]
pub struct MaxAmount(pub U256);

impl PaymentSelector for MaxAmount {
    fn select<'a>(&self, candidates: &'a [PaymentCandidate]) -> Option<&'a PaymentCandidate> {
        candidates.iter().find(|c| c.amount <= self.0)
    }
}
