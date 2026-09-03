//! Client-side payment signing for the EIP-155 "exact" scheme.
//!
//! This module provides [`Eip155ExactClient`] for signing EVM payments,
//! supporting both EIP-3009 (`transferWithAuthorization`) and Permit2
//! transfer methods. The transfer method is selected automatically based
//! on the server's `PaymentRequirements.extra.assetTransferMethod`.
//!
//! # Permit2 Auto-Approve
//!
//! Permit2 payments require a one-time ERC-20 `approve(Permit2, MAX)` before
//! the first payment. By default, the client does **not** manage this — users
//! must approve manually, or the facilitator will reject with
//! `Permit2AllowanceInsufficient`.
//!
//! To enable automatic approval, construct the client with a [`Permit2Approver`]
//! via [`Eip155ExactClientBuilder`]:
//!
//! ```ignore
//! let client = Eip155ExactClient::builder(signer)
//!     .approver(my_provider)
//!     .build();
//! ```
//!
//! When an approver is set, the client checks allowances before each Permit2
//! payment and sends an `approve` transaction if needed, making the experience
//! as seamless as EIP-3009.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use alloy_primitives::{Address, FixedBytes, U256};
use alloy_sol_types::{SolStruct, eip712_domain};
use r402_client::{DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use r402_protocol::error::ClientError;
use r402_protocol::network::ChainId;
use r402_protocol::payment::{Base64Bytes, PaymentRequired, ResourceInfo, UnixTimestamp};
use r402_protocol::scheme::SchemeId;
use rand::{RngExt, rng};

use crate::asset::AssetTransferMethod;
use crate::chain::Eip155ChainReference;
use crate::exact::payload::{TokenPermissions as SolTokenPermissions, Witness as SolWitness};
use crate::exact::{
    Eip155Exact, Eip3009Authorization, Eip3009Payload, ExactPayload, PaymentRequirementsExtra,
    TransferWithAuthorization, payload,
};
#[cfg(feature = "client-provider")]
use crate::permit2::BuiltinPermit2Approver;
use crate::permit2::{PERMIT2_ADDRESS, Permit2Approver};
use crate::signer::SignerLike;

mod permit2;
pub use permit2::{Permit2SigningParams, sign_permit2_authorization};

/// Shared EIP-712 signing parameters for ERC-3009 authorization.
///
/// `extra` is mandatory: ERC-3009 requires the EIP-712 domain to declare the
/// token's `name` and `version` (e.g. `"USDC"` / `"2"`). If either is empty
/// or wrong, the facilitator will reconstruct a different EIP-712 hash and
/// reject the signature with `invalid_exact_evm_payload_signature`. The
/// values come from `paymentRequirements.extra` advertised by the seller.
#[derive(Debug, Clone)]
pub struct Eip3009SigningParams {
    /// The EIP-155 chain ID (numeric)
    pub chain_id: u64,
    /// The token contract address (verifying contract for EIP-712)
    pub asset_address: Address,
    /// The recipient address for the transfer
    pub pay_to: Address,
    /// The amount to transfer
    pub amount: U256,
    /// Maximum timeout in seconds for the authorization validity window
    pub max_timeout_seconds: u64,
    /// EIP-712 domain `name` and `version` for the token. Required by
    /// ERC-3009 and must match what the token contract declares; otherwise
    /// the EIP-712 hash on either side disagrees and the signature fails.
    pub extra: PaymentRequirementsExtra,
}

/// Signs an ERC-3009 `TransferWithAuthorization` using EIP-712.
/// It constructs the EIP-712 domain, builds the authorization struct with appropriate
/// timing parameters, and signs the resulting hash.
///
/// # Errors
///
/// Returns [`ClientError`] if EIP-712 signing fails.
pub async fn sign_erc3009_authorization<S: SignerLike + Sync>(
    signer: &S,
    params: &Eip3009SigningParams,
) -> Result<Eip3009Payload, ClientError> {
    if params.extra.name.is_empty() {
        return Err(ClientError::Signing(
            "ERC-3009 EIP-712 domain `name` is required".into(),
        ));
    }
    if params.extra.version.is_empty() {
        return Err(ClientError::Signing(
            "ERC-3009 EIP-712 domain `version` is required".into(),
        ));
    }
    let name = params.extra.name.clone();
    let version = params.extra.version.clone();

    let domain = eip712_domain! {
        name: name,
        version: version,
        chain_id: params.chain_id,
        verifying_contract: params.asset_address,
    };

    let now = UnixTimestamp::now();
    // valid_after should be in the past (10 minutes ago) to ensure the payment is immediately valid
    let valid_after_secs = now.as_secs().saturating_sub(10 * 60);
    let valid_after = UnixTimestamp::from_secs(valid_after_secs);
    let valid_before = now + params.max_timeout_seconds;
    let nonce: [u8; 32] = rng().random();
    let nonce = FixedBytes(nonce);

    let authorization = Eip3009Authorization {
        from: signer.address(),
        to: params.pay_to,
        value: params.amount.into(),
        valid_after,
        valid_before,
        nonce,
    };

    // IMPORTANT: The values here MUST match the authorization struct exactly,
    // as the facilitator will reconstruct this struct from the authorization
    // to verify the signature.
    let transfer_with_authorization = TransferWithAuthorization {
        from: authorization.from,
        to: authorization.to,
        value: authorization.value.into(),
        validAfter: U256::from(authorization.valid_after.as_secs()),
        validBefore: U256::from(authorization.valid_before.as_secs()),
        nonce: authorization.nonce,
    };

    let eip712_hash = transfer_with_authorization.eip712_signing_hash(&domain);
    let signature = signer
        .sign_hash(&eip712_hash)
        .await
        .map_err(|e| ClientError::Signing(format!("{e:?}")))?;

    Ok(Eip3009Payload {
        signature: signature.as_bytes().into(),
        authorization,
    })
}

/// Client for signing EIP-155 exact scheme payments.
///
/// Supports both EIP-3009 (`transferWithAuthorization`) and Permit2 transfer
/// methods on EVM chains. The transfer method is determined automatically
/// based on the server's `PaymentRequirements.extra.assetTransferMethod`.
///
/// # Construction
///
/// - [`new`](Self::new) — Minimal client for EIP-3009 payments (no provider needed).
/// - [`builder`](Self::builder) — Fluent builder for advanced configuration including
///   Permit2 auto-approve via a [`Permit2Approver`].
///
/// # Permit2 Auto-Approve
///
/// When constructed with a [`Permit2Approver`], the client automatically manages
/// Permit2 ERC-20 allowances:
///
/// - **Without approver** (default): Permit2 payments require the user to have
///   previously called `token.approve(Permit2, MAX)`. If the allowance is
///   insufficient, the facilitator rejects with `Permit2AllowanceInsufficient`.
///
/// - **With approver**: The client checks allowance before each Permit2 payment
///   and automatically sends an `approve` transaction if needed (one-time,
///   ~46k gas). This makes Permit2 as frictionless as EIP-3009.
///
/// # Examples
///
/// ```ignore
/// // Simple: EIP-3009 only, no provider needed
/// let client = Eip155ExactClient::new(signer);
///
/// // With Permit2 auto-approve
/// let client = Eip155ExactClient::builder(signer)
///     .approver(my_provider)
///     .build();
/// ```
pub struct Eip155ExactClient<S> {
    signer: S,
    approver: Option<Arc<dyn Permit2Approver>>,
    auto_approve: bool,
}

impl<S: std::fmt::Debug> std::fmt::Debug for Eip155ExactClient<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Eip155ExactClient")
            .field("signer", &self.signer)
            .field("has_approver", &self.approver.is_some())
            .field("auto_approve", &self.auto_approve)
            .finish()
    }
}

impl<S> Eip155ExactClient<S> {
    /// Creates a new EIP-155 exact scheme client with the given signer.
    ///
    /// This is the simplest construction path. EIP-3009 payments work
    /// immediately; Permit2 payments require the user to have approved
    /// the Permit2 contract manually beforehand.
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
    ///
    /// Use the builder to attach a [`Permit2Approver`] for automatic
    /// allowance management:
    ///
    /// ```ignore
    /// let client = Eip155ExactClient::builder(signer)
    ///     .approver(my_provider)
    ///     .build();
    /// ```
    pub fn builder(signer: S) -> Eip155ExactClientBuilder<S> {
        Eip155ExactClientBuilder {
            signer,
            approver: None,
            auto_approve: true,
        }
    }
}

/// Builder for constructing an [`Eip155ExactClient`] with optional Permit2
/// auto-approve capabilities.
///
/// Created via [`Eip155ExactClient::builder`].
///
/// # Examples
///
/// ```ignore
/// // Minimal (equivalent to Eip155ExactClient::new(signer))
/// let client = Eip155ExactClient::builder(signer).build();
///
/// // With Permit2 auto-approve (recommended for AI agents)
/// let client = Eip155ExactClient::builder(signer)
///     .approver(my_provider)
///     .build();
///
/// // Check-only mode: detect insufficient allowance without auto-approving
/// let client = Eip155ExactClient::builder(signer)
///     .approver(my_provider)
///     .auto_approve(false)
///     .build();
/// ```
pub struct Eip155ExactClientBuilder<S> {
    signer: S,
    approver: Option<Arc<dyn Permit2Approver>>,
    auto_approve: bool,
}

impl<S: std::fmt::Debug> std::fmt::Debug for Eip155ExactClientBuilder<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Eip155ExactClientBuilder")
            .field("signer", &self.signer)
            .field("has_approver", &self.approver.is_some())
            .field("auto_approve", &self.auto_approve)
            .finish()
    }
}

impl<S> Eip155ExactClientBuilder<S> {
    /// Sets the [`Permit2Approver`] for automatic allowance management.
    ///
    /// When set, the client will check Permit2 allowances before signing
    /// and, if [`auto_approve`](Self::auto_approve) is `true` (the default),
    /// automatically send an `approve(Permit2, MAX)` transaction when needed.
    #[must_use]
    pub fn approver<A: Permit2Approver + 'static>(mut self, approver: A) -> Self {
        self.approver = Some(Arc::new(approver));
        self
    }

    /// Controls whether the client automatically sends `approve` transactions
    /// when Permit2 allowance is insufficient.
    ///
    /// - `true` (default): Automatically approve — seamless experience.
    /// - `false`: Return [`ClientError::PreConditionFailed`] with a descriptive
    ///   message, giving callers a chance to handle the approval themselves.
    ///
    /// Has no effect if no [`approver`](Self::approver) is set.
    #[must_use]
    pub const fn auto_approve(mut self, auto_approve: bool) -> Self {
        self.auto_approve = auto_approve;
        self
    }

    /// Attaches an Alloy [`Provider`](alloy_provider::Provider) for automatic
    /// Permit2 allowance management.
    ///
    /// This is the **recommended, batteries-included** way to enable Permit2
    /// auto-approve. The provider must have a wallet/signer configured that
    /// matches the payment signer, so it can send `approve` transactions.
    ///
    /// Internally creates a built-in [`Permit2Approver`] — no trait
    /// implementation required from the caller.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let provider = ProviderBuilder::new()
    ///     .wallet(EthereumWallet::new(signer.clone()))
    ///     .connect_http(rpc_url);
    ///
    /// let client = Eip155ExactClient::builder(signer)
    ///     .provider(provider)
    ///     .build();
    /// ```
    ///
    /// # Feature
    ///
    /// Requires the **`client-provider`** feature flag.
    #[cfg(feature = "client-provider")]
    #[must_use]
    pub fn provider<P: alloy_provider::Provider + Send + Sync + 'static>(
        self,
        provider: P,
    ) -> Self {
        self.approver(BuiltinPermit2Approver { provider })
    }

    /// Builds the configured [`Eip155ExactClient`].
    pub fn build(self) -> Eip155ExactClient<S> {
        Eip155ExactClient {
            signer: self.signer,
            approver: self.approver,
            auto_approve: self.auto_approve,
        }
    }
}

impl<S> SchemeId for Eip155ExactClient<S> {
    fn namespace(&self) -> &str {
        Eip155Exact.namespace()
    }

    fn scheme(&self) -> &str {
        Eip155Exact.scheme()
    }
}

impl<S> SchemeClient for Eip155ExactClient<S>
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
                    signer: Box::new(V2PayloadSigner {
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

    fn find_default_asset(&self, asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
        crate::find_default_evm_asset_info(asset, network)
    }
}

struct V2PayloadSigner<S> {
    signer: S,
    resource_info: Option<ResourceInfo>,
    chain_reference: Eip155ChainReference,
    requirements: payload::v2::PaymentRequirements,
    approver: Option<Arc<dyn Permit2Approver>>,
    auto_approve: bool,
}

impl<S> PaymentCandidateSigner for V2PayloadSigner<S>
where
    S: Sync + SignerLike,
{
    #[allow(
        clippy::excessive_nesting,
        reason = "async signing logic with conditional Permit2 flow"
    )]
    fn sign_payment(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + '_>> {
        Box::pin(async move {
            let use_permit2 = self
                .requirements
                .extra
                .as_ref()
                .and_then(|e| e.asset_transfer_method)
                == Some(AssetTransferMethod::Permit2);

            let exact_payload = if use_permit2 {
                // Auto-approve: ensure Permit2 has sufficient ERC-20 allowance
                // before signing, if a Permit2Approver was provided.
                if let Some(approver) = &self.approver {
                    let token = self.requirements.asset.0;
                    let owner = self.signer.address();
                    let required: U256 = self.requirements.amount.into();

                    let allowance = approver.check_permit2_allowance(token, owner).await?;

                    if allowance < required {
                        if self.auto_approve {
                            approver.approve_permit2(token, owner).await?;
                        } else {
                            return Err(ClientError::PreConditionFailed(format!(
                                "Permit2 allowance insufficient for token {token}: \
                                 have {allowance}, need {required}. \
                                 Call approve({PERMIT2_ADDRESS}, MAX) on the token contract, \
                                 or enable auto_approve in the client builder."
                            )));
                        }
                    }
                }

                let params = Permit2SigningParams {
                    chain_id: self.chain_reference.inner(),
                    asset_address: self.requirements.asset.0,
                    pay_to: self.requirements.pay_to.into(),
                    amount: self.requirements.amount.into(),
                    max_timeout_seconds: self.requirements.max_timeout_seconds,
                };
                let permit2_payload = sign_permit2_authorization(&self.signer, &params).await?;
                ExactPayload::Permit2(permit2_payload)
            } else {
                let extra = self.requirements.extra.clone().ok_or_else(|| {
                    ClientError::Signing(
                        "ERC-3009 payment requires `paymentRequirements.extra` with the \
                         token's EIP-712 domain `name` and `version`"
                            .into(),
                    )
                })?;
                let params = Eip3009SigningParams {
                    chain_id: self.chain_reference.inner(),
                    asset_address: self.requirements.asset.0,
                    pay_to: self.requirements.pay_to.into(),
                    amount: self.requirements.amount.into(),
                    max_timeout_seconds: self.requirements.max_timeout_seconds,
                    extra,
                };
                let eip3009_payload = sign_erc3009_authorization(&self.signer, &params).await?;
                ExactPayload::Eip3009(eip3009_payload)
            };

            let payload =
                payload::v2::PaymentPayload::new(self.requirements.clone(), exact_payload)
                    .with_optional_resource(self.resource_info.clone());
            let json = serde_json::to_vec(&payload)?;
            let b64 = Base64Bytes::encode(&json);

            Ok(b64.to_string())
        })
    }
}
