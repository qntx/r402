//! Filters applied to the candidate list before selection.

use compact_str::CompactString;
use r402_protocol::ChainIdPattern;

use crate::candidate::PaymentCandidate;

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
    #[must_use]
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
    #[must_use]
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
