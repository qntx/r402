//! Client-side scheme abstractions.

use std::fmt::{self, Debug, Formatter};
use std::future::Future;

use compact_str::CompactString;

use crate::chain::{ChainId, ChainIdPattern};
use crate::error::ClientError;
use crate::scheme::sealed::Sealed;
use crate::wire::PaymentRequired;

/// Client-side scheme interface.
///
/// Sealed: only crates inside this workspace may implement it.
pub trait SchemeClient: super::SchemeId + Sealed + Send + Sync {
    /// Examines a 402 response and returns all payment candidates this
    /// client can fulfil.
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate>;
}

/// A single payment option produced by [`SchemeClient::accept`].
pub struct PaymentCandidate {
    /// CAIP-2 chain id of the target chain.
    pub chain_id: ChainId,
    /// Token asset address / mint.
    pub asset: CompactString,
    /// Amount in the token's smallest unit, stringified.
    pub amount: CompactString,
    /// Scheme identifier (`"exact"`, `"upto"`, ...).
    pub scheme: CompactString,
    /// Recipient address.
    pub pay_to: CompactString,
    /// Signer that can produce the authorization.
    pub signer: Box<dyn PaymentCandidateSigner + Send + Sync>,
}

impl Debug for PaymentCandidate {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaymentCandidate")
            .field("chain_id", &self.chain_id)
            .field("asset", &self.asset)
            .field("amount", &self.amount)
            .field("scheme", &self.scheme)
            .field("pay_to", &self.pay_to)
            .finish_non_exhaustive()
    }
}

impl PaymentCandidate {
    /// Signs the candidate, returning the base64-encoded payload.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] when signing fails.
    pub async fn sign(&self) -> Result<String, ClientError> {
        self.signer.sign_payment().await
    }
}

/// Candidate-signer trait. Object-safe on purpose — each candidate
/// carries its own `Box<dyn PaymentCandidateSigner>`.
pub trait PaymentCandidateSigner: Send + Sync {
    /// Produces the signed payment payload (base64-encoded).
    fn sign_payment<'a>(
        &'a self,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + 'a>>;
}

/// Selector that picks the best candidate from a slice.
pub trait PaymentSelector: Send + Sync {
    /// Returns the selected candidate (if any).
    fn select<'a>(&self, candidates: &[&'a PaymentCandidate]) -> Option<&'a PaymentCandidate>;
}

/// `PaymentSelector` that returns the first candidate.
#[derive(Debug, Clone, Copy, Default)]
pub struct FirstMatch;

impl PaymentSelector for FirstMatch {
    fn select<'a>(&self, candidates: &[&'a PaymentCandidate]) -> Option<&'a PaymentCandidate> {
        candidates.first().copied()
    }
}

/// `PaymentSelector` that prefers chains matching one of the given patterns.
#[derive(Debug, Default)]
pub struct PreferChain(Vec<ChainIdPattern>);

impl PreferChain {
    /// Constructs a selector from patterns.
    pub fn new<P: Into<Vec<ChainIdPattern>>>(patterns: P) -> Self {
        Self(patterns.into())
    }

    /// Appends additional patterns with lower priority.
    #[must_use]
    pub fn or_chain<P: Into<Vec<ChainIdPattern>>>(mut self, patterns: P) -> Self {
        self.0.extend(patterns.into());
        self
    }
}

impl PaymentSelector for PreferChain {
    fn select<'a>(&self, candidates: &[&'a PaymentCandidate]) -> Option<&'a PaymentCandidate> {
        for pattern in &self.0 {
            if let Some(hit) = candidates.iter().find(|c| pattern.matches(&c.chain_id)) {
                return Some(*hit);
            }
        }
        candidates.first().copied()
    }
}

/// `PaymentSelector` that rejects candidates whose amount exceeds the budget.
#[derive(Debug, Clone, Copy)]
pub struct MaxAmount(pub u128);

impl PaymentSelector for MaxAmount {
    fn select<'a>(&self, candidates: &[&'a PaymentCandidate]) -> Option<&'a PaymentCandidate> {
        candidates
            .iter()
            .find(|c| c.amount.parse::<u128>().is_ok_and(|a| a <= self.0))
            .copied()
    }
}

/// Filter applied to the candidate list before selection.
pub trait PaymentPolicy: Send + Sync {
    /// Returns the subset of candidates that pass this policy.
    fn apply<'a>(&self, candidates: Vec<&'a PaymentCandidate>) -> Vec<&'a PaymentCandidate>;
}

/// Keeps only candidates whose chain matches one of the patterns.
#[derive(Debug, Default)]
pub struct NetworkPolicy(Vec<ChainIdPattern>);

impl NetworkPolicy {
    /// Constructs the policy from a list of patterns.
    pub fn new<P: Into<Vec<ChainIdPattern>>>(patterns: P) -> Self {
        Self(patterns.into())
    }
}

impl PaymentPolicy for NetworkPolicy {
    fn apply<'a>(&self, candidates: Vec<&'a PaymentCandidate>) -> Vec<&'a PaymentCandidate> {
        candidates
            .into_iter()
            .filter(|c| self.0.iter().any(|p| p.matches(&c.chain_id)))
            .collect()
    }
}

/// Keeps only candidates whose scheme name is in the allow-list.
#[derive(Debug, Default)]
pub struct SchemePolicy(Vec<CompactString>);

impl SchemePolicy {
    /// Constructs the policy from an iterator of names.
    pub fn new<S: Into<CompactString>, I: IntoIterator<Item = S>>(schemes: I) -> Self {
        Self(schemes.into_iter().map(Into::into).collect())
    }
}

impl PaymentPolicy for SchemePolicy {
    fn apply<'a>(&self, candidates: Vec<&'a PaymentCandidate>) -> Vec<&'a PaymentCandidate> {
        candidates
            .into_iter()
            .filter(|c| self.0.iter().any(|s| s.as_str() == c.scheme.as_str()))
            .collect()
    }
}

/// Keeps only candidates whose amount is at most the configured bound.
#[derive(Debug, Clone, Copy)]
pub struct MaxAmountPolicy(pub u128);

impl PaymentPolicy for MaxAmountPolicy {
    fn apply<'a>(&self, candidates: Vec<&'a PaymentCandidate>) -> Vec<&'a PaymentCandidate> {
        candidates
            .into_iter()
            .filter(|c| c.amount.parse::<u128>().is_ok_and(|a| a <= self.0))
            .collect()
    }
}
