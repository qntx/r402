//! Client-side payment signing for the EIP-155 "upto" scheme.
//!
//! [`Eip155UptoClient`] produces Permit2 `PermitWitnessTransferFrom`
//! signatures authorising **up to** the amount declared in the server's
//! price tag. The facilitator later settles for any amount in `[0, max]`.
//!
//! # Permit2 auto-approve
//!
//! The upto scheme shares Permit2 plumbing with exact, so the same
//! [`Permit2Approver`] trait powers both: attach one via
//! [`Eip155UptoClientBuilder::approver`] (or
//! [`provider`](Eip155UptoClientBuilder::provider) behind the
//! `client-provider` feature) to enable hands-off `approve(Permit2, MAX)`
//! before the first signature.

pub mod eip2612;
mod signing;

use std::sync::Arc;

use alloy_primitives::U256;
pub use eip2612::{Eip2612SigningParams, sign_eip2612_permit};
use r402_core::error::ClientError;
use r402_core::scheme::{PaymentCandidate, PaymentCandidateSigner, SchemeClient, SchemeId, Sealed};
use r402_core::wire::{Base64Bytes, PaymentRequired, ResourceInfo};
pub use signing::{Permit2UptoSigningParams, sign_permit2_upto_authorization};

use crate::chain::Eip155ChainReference;
use crate::permit2::{PERMIT2_ADDRESS, Permit2Approver};
use crate::signer::SignerLike;
use crate::upto::{Eip155Upto, types};

/// Client for signing EIP-155 upto scheme payments (Permit2 only).
///
/// # Construction
///
/// - [`Eip155UptoClient::new`] — minimal client with no Permit2 allowance
///   management; the buyer MUST have pre-approved Permit2 for the token.
/// - [`Eip155UptoClient::builder`] — fluent builder for advanced
///   configuration including Permit2 auto-approve.
///
/// # Example
///
/// ```ignore
/// use r402_evm::Eip155UptoClient;
///
/// let client = Eip155UptoClient::new(signer);
/// let candidates = client.accept(&payment_required_response);
/// let payload = candidates[0].sign().await?;
/// ```
pub struct Eip155UptoClient<S> {
    signer: S,
    approver: Option<Arc<dyn Permit2Approver>>,
    auto_approve: bool,
}

impl<S: std::fmt::Debug> std::fmt::Debug for Eip155UptoClient<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Eip155UptoClient")
            .field("signer", &self.signer)
            .field("has_approver", &self.approver.is_some())
            .field("auto_approve", &self.auto_approve)
            .finish()
    }
}

impl<S> Eip155UptoClient<S> {
    /// Creates a new EIP-155 upto scheme client with the given signer.
    ///
    /// For automatic Permit2 approval, use [`builder`](Self::builder) instead.
    pub const fn new(signer: S) -> Self {
        Self {
            signer,
            approver: None,
            auto_approve: false,
        }
    }

    /// Returns a builder for advanced client configuration.
    pub fn builder(signer: S) -> Eip155UptoClientBuilder<S> {
        Eip155UptoClientBuilder {
            signer,
            approver: None,
            auto_approve: true,
        }
    }
}

/// Builder for [`Eip155UptoClient`] with optional Permit2 auto-approve.
pub struct Eip155UptoClientBuilder<S> {
    signer: S,
    approver: Option<Arc<dyn Permit2Approver>>,
    auto_approve: bool,
}

impl<S: std::fmt::Debug> std::fmt::Debug for Eip155UptoClientBuilder<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Eip155UptoClientBuilder")
            .field("signer", &self.signer)
            .field("has_approver", &self.approver.is_some())
            .field("auto_approve", &self.auto_approve)
            .finish()
    }
}

impl<S> Eip155UptoClientBuilder<S> {
    /// Attaches a custom [`Permit2Approver`] implementation.
    #[must_use]
    pub fn approver<A: Permit2Approver + 'static>(mut self, approver: A) -> Self {
        self.approver = Some(Arc::new(approver));
        self
    }

    /// Controls whether the client sends `approve` transactions automatically
    /// when the ERC-20 allowance on Permit2 is insufficient.
    ///
    /// - `true` (default when using the builder) — auto-approve, seamless UX.
    /// - `false` — return [`ClientError::PreConditionFailed`] with a
    ///   descriptive message, letting callers orchestrate approval themselves.
    ///
    /// Has no effect without an [`approver`](Self::approver) attached.
    #[must_use]
    pub const fn auto_approve(mut self, auto_approve: bool) -> Self {
        self.auto_approve = auto_approve;
        self
    }

    /// Attaches an Alloy [`Provider`](alloy_provider::Provider) as a Permit2
    /// approver in one step — the batteries-included auto-approve path.
    ///
    /// Requires the `client-provider` feature.
    #[cfg(feature = "client-provider")]
    #[must_use]
    pub fn provider<P: alloy_provider::Provider + Send + Sync + 'static>(
        self,
        provider: P,
    ) -> Self {
        // Re-use the exact scheme's built-in approver; the shape is identical
        // (check_permit2_allowance + approve_permit2).
        self.approver(crate::permit2::BuiltinPermit2Approver { provider })
    }

    /// Builds the configured [`Eip155UptoClient`].
    pub fn build(self) -> Eip155UptoClient<S> {
        Eip155UptoClient {
            signer: self.signer,
            approver: self.approver,
            auto_approve: self.auto_approve,
        }
    }
}

impl<S> SchemeId for Eip155UptoClient<S> {
    fn namespace(&self) -> &str {
        Eip155Upto.namespace()
    }

    fn scheme(&self) -> &str {
        Eip155Upto.scheme()
    }
}

impl<S> Sealed for Eip155UptoClient<S> {}

impl<S> SchemeClient for Eip155UptoClient<S>
where
    S: SignerLike + Clone + Send + Sync + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: types::v2::PaymentRequirements = v.as_concrete()?;
                let chain_reference = Eip155ChainReference::try_from(&requirements.network).ok()?;
                let candidate = PaymentCandidate {
                    chain_id: requirements.network.clone(),
                    asset: requirements.asset.to_string().into(),
                    amount: requirements.amount.0.to_string().into(),
                    scheme: self.scheme().into(),
                    pay_to: requirements.pay_to.to_string().into(),
                    signer: Box::new(UptoPayloadSigner {
                        resource_info: Some(payment_required.resource.clone()),
                        signer: self.signer.clone(),
                        chain_reference,
                        requirements,
                        approver: self.approver.clone(),
                        auto_approve: self.auto_approve,
                    }),
                };
                Some(candidate)
            })
            .collect::<Vec<_>>()
    }
}

struct UptoPayloadSigner<S> {
    signer: S,
    resource_info: Option<ResourceInfo>,
    chain_reference: Eip155ChainReference,
    requirements: types::v2::PaymentRequirements,
    approver: Option<Arc<dyn Permit2Approver>>,
    auto_approve: bool,
}

impl<S> UptoPayloadSigner<S>
where
    S: Sync + SignerLike,
{
    /// Ensures the signer's ERC-20 allowance on Permit2 covers the declared
    /// authorised maximum. Returns early when no approver is configured.
    async fn ensure_permit2_allowance(&self) -> Result<(), ClientError> {
        let Some(approver) = &self.approver else {
            return Ok(());
        };
        let token = self.requirements.asset.0;
        let owner = self.signer.address();
        let required: U256 = self.requirements.amount.into();

        let allowance = approver.check_permit2_allowance(token, owner).await?;
        if allowance >= required {
            return Ok(());
        }
        if self.auto_approve {
            approver.approve_permit2(token, owner).await?;
            Ok(())
        } else {
            Err(ClientError::PreConditionFailed(format!(
                "Permit2 allowance insufficient for token {token}: \
                 have {allowance}, need {required}. Call approve({PERMIT2_ADDRESS}, MAX) \
                 on the token contract, or enable auto_approve."
            )))
        }
    }
}

impl<S> PaymentCandidateSigner for UptoPayloadSigner<S>
where
    S: Sync + SignerLike,
{
    fn sign_payment(&self) -> r402_core::facilitator::BoxFuture<'_, Result<String, ClientError>> {
        Box::pin(async move {
            self.ensure_permit2_allowance().await?;

            // Spec §2: the buyer MUST embed `paymentRequirements.extra.facilitatorAddress`
            // into `witness.facilitator`. Reject early when the server failed
            // to publish one — we cannot produce a settle-able signature.
            let extra = self.requirements.extra.as_ref().ok_or_else(|| {
                ClientError::PreConditionFailed(
                    "upto requires paymentRequirements.extra.facilitatorAddress; \
                     ensure the server is wired to an upto facilitator that publishes its address"
                        .to_owned(),
                )
            })?;
            let params = Permit2UptoSigningParams {
                chain_id: self.chain_reference.inner(),
                asset_address: self.requirements.asset.0,
                pay_to: self.requirements.pay_to.into(),
                facilitator_address: extra.facilitator_address.0,
                max_amount: self.requirements.amount.into(),
                max_timeout_seconds: self.requirements.max_timeout_seconds,
            };
            let upto_payload = sign_permit2_upto_authorization(&self.signer, &params).await?;

            let payload = types::v2::PaymentPayload::new(self.requirements.clone(), upto_payload)
                .with_optional_resource(self.resource_info.clone());
            let json = serde_json::to_vec(&payload)?;
            let b64 = Base64Bytes::encode(&json);

            Ok(b64.to_string())
        })
    }
}
