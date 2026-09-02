//! Candidate selection and `extra.paymentFlow` preference.

use r402_protocol::{ChainIdPattern, ClientError, PaymentRequired, PaymentRequirements};

use crate::candidate::PaymentCandidate;
use crate::register::PaymentClient;

/// Selector that picks the best candidate from a slice.
pub trait PaymentSelector: Send + Sync {
    /// Returns the selected candidate (if any).
    fn select<'a>(&self, candidates: &[&'a PaymentCandidate]) -> Option<&'a PaymentCandidate>;
}

/// [`PaymentSelector`] that returns the first candidate.
#[derive(Debug, Clone, Copy, Default)]
pub struct FirstMatch;

impl PaymentSelector for FirstMatch {
    fn select<'a>(&self, candidates: &[&'a PaymentCandidate]) -> Option<&'a PaymentCandidate> {
        candidates.first().copied()
    }
}

/// [`PaymentSelector`] that prefers chains matching one of the given patterns.
#[derive(Debug, Default)]
pub struct PreferChain(Vec<ChainIdPattern>);

impl PreferChain {
    /// Constructs a selector from patterns.
    #[must_use]
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

/// [`PaymentSelector`] that rejects candidates whose amount exceeds the budget.
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

impl<S: PaymentSelector> PaymentClient<S> {
    /// Collects candidates from every registered scheme client.
    #[must_use]
    pub fn candidates(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        let mut out = Vec::new();
        for client in &self.schemes {
            out.extend(client.accept(payment_required));
        }
        out
    }

    /// Applies spend controls, policies, and the selector.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::NoMatchingPaymentOption`] when nothing remains,
    /// [`ClientError::UnrecognizedPaymentFlow`] when every candidate has an
    /// unknown `extra.paymentFlow`, or [`ClientError::SpendControls`] when
    /// spend controls reject every option.
    pub fn select_candidate<'a>(
        &self,
        candidates: &'a [PaymentCandidate],
    ) -> Result<&'a PaymentCandidate, ClientError> {
        let recognized: Vec<&PaymentCandidate> = candidates
            .iter()
            .filter(|candidate| is_recognized_payment_flow(&candidate.requirements))
            .collect();
        if recognized.is_empty() {
            return Err(if candidates.is_empty() {
                ClientError::NoMatchingPaymentOption
            } else {
                ClientError::UnrecognizedPaymentFlow
            });
        }

        let mut filtered = self.apply_spend_controls(recognized)?;
        for policy in &self.policies {
            filtered = policy.apply(filtered);
            if filtered.is_empty() {
                return Err(ClientError::NoMatchingPaymentOption);
            }
        }

        let preferred = prefer_authorization(filtered);
        self.selector
            .select(&preferred)
            .ok_or(ClientError::NoMatchingPaymentOption)
    }
}

fn prefer_authorization(candidates: Vec<&PaymentCandidate>) -> Vec<&PaymentCandidate> {
    let authorization: Vec<&PaymentCandidate> = candidates
        .iter()
        .copied()
        .filter(|candidate| is_authorization_payment_flow(&candidate.requirements))
        .collect();
    if authorization.is_empty() {
        candidates
    } else {
        authorization
    }
}

#[derive(Clone, Copy)]
enum WirePaymentFlow {
    Absent,
    Authorization,
    Upfront,
    Escrow,
    Unknown,
}

fn wire_payment_flow(requirements: &PaymentRequirements) -> WirePaymentFlow {
    let Some(flow) = requirements
        .extra
        .as_ref()
        .and_then(|extra| extra.get("paymentFlow"))
    else {
        return WirePaymentFlow::Absent;
    };
    if flow.is_null() {
        return WirePaymentFlow::Absent;
    }
    match flow.as_str() {
        Some("authorization") => WirePaymentFlow::Authorization,
        Some("upfront") => WirePaymentFlow::Upfront,
        Some("escrow") => WirePaymentFlow::Escrow,
        _ => WirePaymentFlow::Unknown,
    }
}

fn is_recognized_payment_flow(requirements: &PaymentRequirements) -> bool {
    !matches!(wire_payment_flow(requirements), WirePaymentFlow::Unknown)
}

fn is_authorization_payment_flow(requirements: &PaymentRequirements) -> bool {
    matches!(
        wire_payment_flow(requirements),
        WirePaymentFlow::Absent | WirePaymentFlow::Authorization
    )
}
