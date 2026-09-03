//! Client-side payment signing for the EIP-155 "upto" scheme.
//!
//! Produces Permit2 `PermitWitnessTransferFrom` signatures authorising **up to**
//! the amount declared in the server's price tag.

pub mod eip2612;
mod signing;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use alloy_primitives::U256;
pub use eip2612::{Eip2612SigningParams, sign_eip2612_permit};
use r402_client::{DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use r402_protocol::error::ClientError;
use r402_protocol::network::ChainId;
use r402_protocol::payment::{Base64Bytes, Extensions, PaymentRequired, ResourceInfo};
use r402_protocol::scheme::SchemeId;
pub use signing::{Permit2UptoSigningParams, sign_permit2_upto_authorization};

use crate::chain::Eip155ChainReference;
#[cfg(feature = "client-provider")]
use crate::permit2::BuiltinPermit2Approver;
use crate::permit2::{PERMIT2_ADDRESS, Permit2Approver};
use crate::signer::SignerLike;
use crate::upto::{Eip155Upto, payload};

/// Client for signing EIP-155 upto scheme payments (Permit2 only).
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
    /// Creates a client with no Permit2 allowance management.
    pub const fn new(signer: S) -> Self {
        Self {
            signer,
            approver: None,
            auto_approve: false,
        }
    }

    /// Returns a builder for Permit2 auto-approve configuration.
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

    /// Controls whether the client sends `approve` when allowance is insufficient.
    #[must_use]
    pub const fn auto_approve(mut self, auto_approve: bool) -> Self {
        self.auto_approve = auto_approve;
        self
    }

    /// Attaches an Alloy provider as a Permit2 approver.
    #[cfg(feature = "client-provider")]
    #[must_use]
    pub fn provider<P: alloy_provider::Provider + Send + Sync + 'static>(
        self,
        provider: P,
    ) -> Self {
        self.approver(BuiltinPermit2Approver { provider })
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

impl<S> SchemeClient for Eip155UptoClient<S>
where
    S: SignerLike + Clone + Send + Sync + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: payload::v2::PaymentRequirements = v.as_concrete()?;
                let chain_reference = Eip155ChainReference::try_from(&requirements.network).ok()?;
                let candidate = PaymentCandidate {
                    chain_id: requirements.network.clone(),
                    asset: requirements.asset.to_string().into(),
                    amount: requirements.amount.0.to_string().into(),
                    scheme: self.scheme().into(),
                    pay_to: requirements.pay_to.to_string().into(),
                    requirements: v.clone(),
                    signer: Box::new(UptoPayloadSigner {
                        resource_info: Some(payment_required.resource.clone()),
                        signer: self.signer.clone(),
                        chain_reference,
                        requirements,
                        approver: self.approver.clone(),
                        auto_approve: self.auto_approve,
                        advertised: payment_required.extensions.clone(),
                    }),
                };
                Some(candidate)
            })
            .collect::<Vec<_>>()
    }

    fn find_default_asset(&self, asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
        crate::find_default_evm_asset_info(asset, network)
    }
}

struct UptoPayloadSigner<S> {
    signer: S,
    resource_info: Option<ResourceInfo>,
    chain_reference: Eip155ChainReference,
    requirements: payload::v2::PaymentRequirements,
    approver: Option<Arc<dyn Permit2Approver>>,
    auto_approve: bool,
    advertised: Extensions,
}

impl<S> UptoPayloadSigner<S>
where
    S: Sync + SignerLike,
{
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
    fn sign_payment(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + '_>> {
        Box::pin(async move {
            // Buyer MUST embed extra.facilitatorAddress into witness.facilitator.
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
            let name = extra.name.as_str();
            let version = extra.version.as_str();
            let required: U256 = upto_payload.permit2_authorization.permitted.amount.into();
            let deadline: U256 = upto_payload.permit2_authorization.deadline.into();

            let mut payload =
                payload::v2::PaymentPayload::new(self.requirements.clone(), upto_payload)
                    .with_optional_resource(self.resource_info.clone());
            if let Some(permit) = eip2612::try_sign_eip2612_permit_extension(
                &self.signer,
                self.approver.as_ref(),
                &self.advertised,
                name,
                version,
                self.chain_reference.inner(),
                self.requirements.asset.0,
                required,
                deadline,
            )
            .await?
            {
                payload.extensions.insert(
                    crate::eip2612::EIP2612_GAS_SPONSORING_KEY,
                    permit.to_extension_entry()?,
                );
            } else if let Some(info) = eip2612::try_sign_erc20_approval_extension(
                &self.signer,
                self.approver.as_ref(),
                &self.advertised,
                self.chain_reference.inner(),
                self.requirements.asset.0,
                required,
            )
            .await?
            {
                payload.extensions.insert(
                    crate::erc20_approval::ERC20_APPROVAL_GAS_SPONSORING_KEY,
                    info.to_extension_entry()?,
                );
            }
            if payload.extensions.is_empty() {
                self.ensure_permit2_allowance().await?;
            }
            let json = serde_json::to_vec(&payload)?;
            let b64 = Base64Bytes::encode(&json);

            Ok(b64.to_string())
        })
    }
}
