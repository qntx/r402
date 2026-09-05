//! Client-side payment signing for the EIP-155 "exact" scheme.
//!
//! This module provides [`Eip155ExactClient`] for signing EVM payments,
//! supporting both EIP-3009 (`transferWithAuthorization`) and Permit2
//! transfer methods. The transfer method is selected automatically based
//! on the server's `PaymentRequirements.extra.assetTransferMethod`.
//!
//! # Permit2 allowance
//!
//! 1. The 402 must advertise `eip2612GasSponsoring` and/or
//!    `erc20ApprovalGasSponsoring` (HTTP auto-register).
//! 2. Build with a provider so the client can `readContract` and attach:
//!
//! ```ignore
//! let client = Eip155UptoClient::builder(signer)
//!     .provider(alloy_provider)
//!     .auto_approve(false)
//!     .build();
//! ```
//!
//! [`Eip155UptoClient`](crate::upto::Eip155UptoClient) and
//! [`Eip155ExactClient::builder`] share this construction.
//!
//! 3. [`auto_approve`](Eip155ExactClientBuilder::auto_approve) (default `true`)
//!    is spec Option A (buyer-paid `approve(Permit2, MAX)`) and **only runs if
//!    no sponsoring extension was attached**.
//! 4. [`new`](Eip155ExactClient::new) cannot `readContract`, so it cannot
//!    attach sponsoring. The first Permit2 payment is HTTP 412
//!    (`Permit2AllowanceRequired`). EIP-3009 does not need a provider.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use alloy_primitives::{Address, FixedBytes, U256};
use alloy_sol_types::{SolStruct, eip712_domain};
use r402_client::{DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use r402_protocol::error::ClientError;
use r402_protocol::network::ChainId;
use r402_protocol::payment::{
    Base64Bytes, Extensions, PaymentRequired, ResourceInfo, UnixTimestamp,
};
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
/// - [`new`](Self::new) — EIP-3009. No provider; cannot attach Permit2 sponsoring.
/// - [`builder`](Self::builder) — Permit2: provider + advertised
///   `eip2612GasSponsoring` / `erc20ApprovalGasSponsoring`.
///
/// # Permit2 allowance
///
/// See the [module-level Permit2 allowance](crate::exact::client#permit2-allowance).
/// Gasless attach needs a 402 that advertises those extensions (HTTP
/// auto-register) and a provider for `readContract`.
/// [`auto_approve`](Eip155ExactClientBuilder::auto_approve) (default `true`)
/// is spec Option A and **only runs if no sponsoring extension was attached**.
///
/// # Examples
///
/// ```ignore
/// let client = Eip155ExactClient::new(signer);
///
/// let client = Eip155ExactClient::builder(signer)
///     .provider(alloy_provider)
///     .auto_approve(false)
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
    /// Creates a client with no RPC provider.
    ///
    /// EIP-3009 payments work immediately. Permit2 cannot attach
    /// `eip2612GasSponsoring` / `erc20ApprovalGasSponsoring` (no
    /// `readContract`); the first Permit2 payment is HTTP 412
    /// (`Permit2AllowanceRequired`).
    ///
    /// For Permit2, use [`builder`](Self::builder) with a provider.
    pub const fn new(signer: S) -> Self {
        Self {
            signer,
            approver: None,
            auto_approve: false,
        }
    }

    /// Builder for a provider (gasless Permit2 attach) and Option A auto-approve.
    ///
    /// ```ignore
    /// let client = Eip155ExactClient::builder(signer)
    ///     .provider(alloy_provider)
    ///     .auto_approve(false)
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

/// Builder for [`Eip155ExactClient`].
///
/// Created via [`Eip155ExactClient::builder`]. Attach a provider so Permit2
/// can `readContract` and sign advertised `eip2612GasSponsoring` /
/// `erc20ApprovalGasSponsoring`. [`auto_approve`](Self::auto_approve) (default
/// `true`) is spec Option A and **only runs if no sponsoring extension was
/// attached**.
///
/// # Examples
///
/// ```ignore
/// let client = Eip155ExactClient::builder(signer)
///     .provider(alloy_provider)
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
    /// Sets the [`Permit2Approver`] used for `readContract` and Option A approve.
    ///
    /// Gasless attach needs [`Permit2Approver::supports_gas_sponsoring_rpc`].
    /// Prefer `.provider(...)` when the `client-provider` feature is on.
    #[must_use]
    pub fn approver<A: Permit2Approver + 'static>(mut self, approver: A) -> Self {
        self.approver = Some(Arc::new(approver));
        self
    }

    /// Spec Option A: buyer-paid `approve(Permit2, MAX)` when allowance is short.
    ///
    /// Builder default is `true`. **No-op if a sponsoring extension was already
    /// attached.** `false` returns [`ClientError::PreConditionFailed`] instead.
    /// Has no effect without an [`approver`](Self::approver).
    #[must_use]
    pub const fn auto_approve(mut self, auto_approve: bool) -> Self {
        self.auto_approve = auto_approve;
        self
    }

    /// Alloy provider for `readContract` (sponsoring attach) and Option A approve.
    ///
    /// Option A (`auto_approve(true)`) needs a wallet on the provider that
    /// matches the payment signer.
    ///
    /// ```ignore
    /// let client = Eip155ExactClient::builder(signer)
    ///     .provider(alloy_provider)
    ///     .auto_approve(false)
    ///     .build();
    /// ```
    ///
    /// Requires **`client-provider`**.
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

struct V2PayloadSigner<S> {
    signer: S,
    resource_info: Option<ResourceInfo>,
    chain_reference: Eip155ChainReference,
    requirements: payload::v2::PaymentRequirements,
    approver: Option<Arc<dyn Permit2Approver>>,
    auto_approve: bool,
    advertised: Extensions,
}

impl<S> V2PayloadSigner<S>
where
    S: Sync + SignerLike,
{
    async fn ensure_permit2_allowance(
        signer: &S,
        approver: Option<&Arc<dyn Permit2Approver>>,
        auto_approve: bool,
        token: Address,
        required: U256,
    ) -> Result<(), ClientError> {
        let Some(approver) = approver else {
            return Ok(());
        };
        let owner = signer.address();
        let allowance = approver.check_permit2_allowance(token, owner).await?;
        if allowance >= required {
            return Ok(());
        }
        if auto_approve {
            approver.approve_permit2(token, owner).await?;
            Ok(())
        } else {
            Err(ClientError::PreConditionFailed(format!(
                "Permit2 allowance insufficient for token {token}: \
                 have {allowance}, need {required}. \
                 Call approve({PERMIT2_ADDRESS}, MAX) on the token contract, \
                 or enable auto_approve in the client builder."
            )))
        }
    }
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
                let params = Permit2SigningParams {
                    chain_id: self.chain_reference.inner(),
                    asset_address: self.requirements.asset.0,
                    pay_to: self.requirements.pay_to.into(),
                    amount: self.requirements.amount.into(),
                    max_timeout_seconds: self.requirements.max_timeout_seconds,
                };
                let permit2_payload = sign_permit2_authorization(&self.signer, &params).await?;
                let extra = self.requirements.extra.as_ref();
                let name = extra.map_or("", |e| e.name.as_str());
                let version = extra.map_or("", |e| e.version.as_str());
                let required: U256 = permit2_payload
                    .permit2_authorization
                    .permitted
                    .amount
                    .into();
                let deadline: U256 = permit2_payload.permit2_authorization.deadline.into();
                let mut payload = payload::v2::PaymentPayload::new(
                    self.requirements.clone(),
                    ExactPayload::Permit2(permit2_payload),
                )
                .with_optional_resource(self.resource_info.clone());
                if let Some(permit) =
                    crate::upto::client::eip2612::try_sign_eip2612_permit_extension(
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
                } else if let Some(info) =
                    crate::upto::client::eip2612::try_sign_erc20_approval_extension(
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
                    Self::ensure_permit2_allowance(
                        &self.signer,
                        self.approver.as_ref(),
                        self.auto_approve,
                        self.requirements.asset.0,
                        required,
                    )
                    .await?;
                }
                let json = serde_json::to_vec(&payload)?;
                let b64 = Base64Bytes::encode(&json);
                return Ok(b64.to_string());
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
