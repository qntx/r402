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

use alloy_primitives::{Address, Bytes, FixedBytes, Signature, U256};
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{SolCall, SolStruct, eip712_domain, sol};
use r402::proto::Base64Bytes;
use r402::proto::PaymentRequired;
use r402::proto::UnixTimestamp;
use r402::proto::v2::{self, ResourceInfo};
use r402::scheme::SchemeId;
use r402::scheme::{ClientError, PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use rand::RngExt;
use rand::rng;

use crate::chain::Eip155ChainReference;
use crate::chain::TokenAmount;
use crate::exact::types;
use crate::exact::types::{TokenPermissions as SolTokenPermissions, Witness as SolWitness};
use crate::exact::{
    AssetTransferMethod, Eip155Exact, Eip3009Authorization, Eip3009Payload, ExactPayload,
    PERMIT2_ADDRESS, PaymentRequirementsExtra, Permit2Authorization, Permit2Payload,
    Permit2TokenPermissions, Permit2Witness, PermitWitnessTransferFrom, TransferWithAuthorization,
    X402_EXACT_PERMIT2_PROXY,
};

/// A trait that abstracts signing operations, allowing both owned signers and Arc-wrapped signers.
///
/// This is necessary because Alloy's `Signer` trait is not implemented for `Arc<T>`,
/// but users may want to share signers via `Arc` (especially when `PrivateKeySigner` doesn't implement `Clone`).
pub trait SignerLike: Send + Sync {
    /// Returns the address of the signer.
    fn address(&self) -> Address;

    /// Signs the given hash.
    fn sign_hash(
        &self,
        hash: &FixedBytes<32>,
    ) -> impl Future<Output = Result<Signature, alloy_signer::Error>> + Send;
}

impl SignerLike for PrivateKeySigner {
    fn address(&self) -> Address {
        Self::address(self)
    }

    async fn sign_hash(&self, hash: &FixedBytes<32>) -> Result<Signature, alloy_signer::Error> {
        alloy_signer::Signer::sign_hash(self, hash).await
    }
}

impl<T: SignerLike + Send + Sync> SignerLike for Arc<T> {
    fn address(&self) -> Address {
        (**self).address()
    }

    async fn sign_hash(&self, hash: &FixedBytes<32>) -> Result<Signature, alloy_signer::Error> {
        (**self).sign_hash(hash).await
    }
}

/// Abstraction for on-chain interactions needed by the Permit2 auto-approve flow.
///
/// Implement this trait to enable automatic Permit2 allowance management
/// in [`Eip155ExactClient`]. When an approver is provided via
/// [`Eip155ExactClientBuilder::approver`], the client will:
///
/// 1. **Before each Permit2 payment**, call [`check_permit2_allowance`](Self::check_permit2_allowance)
///    to query the current ERC-20 allowance granted to the canonical Permit2 contract.
/// 2. **If the allowance is insufficient**, call [`approve_permit2`](Self::approve_permit2)
///    to send an on-chain `approve(Permit2, MAX_UINT256)` transaction.
/// 3. **Proceed with normal Permit2 EIP-712 signing** once allowance is confirmed.
///
/// This eliminates the manual approve step that Permit2 otherwise requires,
/// giving users the same zero-friction experience as EIP-3009 tokens (like USDC).
///
/// # Gas Costs
///
/// The `approve` transaction costs approximately 46,000 gas (~$0.01–0.10
/// depending on the chain). This cost is borne by the token owner (the
/// signing wallet) and only occurs **once per token** — subsequent payments
/// reuse the existing unlimited allowance.
///
/// # Calldata Helpers
///
/// If you are building a custom implementation, the helper functions
/// [`permit2_allowance_calldata`] and [`permit2_approval_calldata`] generate
/// the raw ABI-encoded calldata for the underlying `eth_call` / `eth_sendTransaction`.
///
/// # Example
///
/// ```ignore
/// use r402_evm::exact::client::{Eip155ExactClient, Permit2Approver};
///
/// struct MyProvider { /* RPC + signer */ }
///
/// impl Permit2Approver for MyProvider {
///     fn check_permit2_allowance(&self, token: Address, owner: Address)
///         -> Pin<Box<dyn Future<Output = Result<U256, ClientError>> + Send + '_>>
///     {
///         Box::pin(async move { /* eth_call: token.allowance(owner, PERMIT2) */ todo!() })
///     }
///
///     fn approve_permit2(&self, token: Address, owner: Address)
///         -> Pin<Box<dyn Future<Output = Result<(), ClientError>> + Send + '_>>
///     {
///         Box::pin(async move { /* send tx: token.approve(PERMIT2, MAX) */ todo!() })
///     }
/// }
///
/// let client = Eip155ExactClient::builder(signer)
///     .approver(MyProvider { /* ... */ })
///     .build();
/// ```
pub trait Permit2Approver: Send + Sync {
    /// Queries the current ERC-20 allowance that `owner` has granted to the
    /// canonical Permit2 contract for the given `token`.
    ///
    /// Equivalent to `token.allowance(owner, PERMIT2_ADDRESS)` via `eth_call`
    /// (read-only, no gas cost).
    fn check_permit2_allowance(
        &self,
        token: Address,
        owner: Address,
    ) -> Pin<Box<dyn Future<Output = Result<U256, ClientError>> + Send + '_>>;

    /// Sends an ERC-20 `approve(PERMIT2_ADDRESS, MAX_UINT256)` transaction
    /// for `token` on behalf of `owner`, and waits for on-chain confirmation.
    ///
    /// This is a **write** operation that costs gas. Implementations should
    /// ensure the transaction is sent from the `owner` address.
    fn approve_permit2(
        &self,
        token: Address,
        owner: Address,
    ) -> Pin<Box<dyn Future<Output = Result<(), ClientError>> + Send + '_>>;
}

/// Shared EIP-712 signing parameters for ERC-3009 authorization.
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
    /// Optional EIP-712 domain name and version override
    pub extra: Option<PaymentRequirementsExtra>,
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
    let (name, version) = params.extra.as_ref().map_or_else(
        || (String::new(), String::new()),
        |extra| (extra.name.clone(), extra.version.clone()),
    );

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
        .map_err(|e| ClientError::SigningError(format!("{e:?}")))?;

    Ok(Eip3009Payload {
        signature: signature.as_bytes().into(),
        authorization,
    })
}

/// Shared signing parameters for Permit2 authorization.
#[derive(Debug, Clone, Copy)]
pub struct Permit2SigningParams {
    /// The EIP-155 chain ID (numeric)
    pub chain_id: u64,
    /// The token contract address
    pub asset_address: Address,
    /// The recipient address for the transfer
    pub pay_to: Address,
    /// The amount to transfer (in token units)
    pub amount: U256,
    /// Maximum timeout in seconds for the authorization validity window
    pub max_timeout_seconds: u64,
}

/// Signs a Permit2 `PermitWitnessTransferFrom` using EIP-712.
///
/// Constructs the Permit2 EIP-712 domain (name = "Permit2", no version,
/// verifying contract = canonical Permit2 address), builds the authorization
/// with timing parameters, and signs the resulting hash.
///
/// # Errors
///
/// Returns [`ClientError`] if EIP-712 signing fails.
pub async fn sign_permit2_authorization<S: SignerLike + Sync>(
    signer: &S,
    params: &Permit2SigningParams,
) -> Result<Permit2Payload, ClientError> {
    let domain = eip712_domain! {
        name: "Permit2",
        chain_id: params.chain_id,
        verifying_contract: PERMIT2_ADDRESS,
    };

    let now = UnixTimestamp::now();
    let valid_after_secs = now.as_secs().saturating_sub(10 * 60);
    let deadline_secs = now.as_secs() + params.max_timeout_seconds;

    // Permit2 uses uint256 nonce (random 32 bytes interpreted as uint256)
    let nonce_bytes: [u8; 32] = rng().random();
    let nonce = U256::from_be_bytes(nonce_bytes);

    let permit_witness = PermitWitnessTransferFrom {
        permitted: SolTokenPermissions {
            token: params.asset_address,
            amount: params.amount,
        },
        spender: X402_EXACT_PERMIT2_PROXY,
        nonce,
        deadline: U256::from(deadline_secs),
        witness: SolWitness {
            to: params.pay_to,
            validAfter: U256::from(valid_after_secs),
            extra: Bytes::new(),
        },
    };

    let eip712_hash = permit_witness.eip712_signing_hash(&domain);
    let signature = signer
        .sign_hash(&eip712_hash)
        .await
        .map_err(|e| ClientError::SigningError(format!("{e:?}")))?;

    let authorization = Permit2Authorization {
        from: signer.address(),
        permitted: Permit2TokenPermissions {
            token: params.asset_address,
            amount: TokenAmount::from(params.amount),
        },
        spender: X402_EXACT_PERMIT2_PROXY,
        nonce: TokenAmount::from(nonce),
        deadline: TokenAmount::from(U256::from(deadline_secs)),
        witness: Permit2Witness {
            to: params.pay_to,
            valid_after: TokenAmount::from(U256::from(valid_after_secs)),
            extra: Bytes::new(),
        },
    };

    Ok(Permit2Payload {
        signature: signature.as_bytes().into(),
        permit2_authorization: authorization,
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

/// Built-in [`Permit2Approver`] backed by an Alloy provider.
///
/// Created automatically when calling
/// [`Eip155ExactClientBuilder::provider`]. Users never need to interact
/// with this type directly.
#[cfg(feature = "client-provider")]
struct BuiltinPermit2Approver<P> {
    provider: P,
}

#[cfg(feature = "client-provider")]
impl<P> Permit2Approver for BuiltinPermit2Approver<P>
where
    P: alloy_provider::Provider + Send + Sync,
{
    fn check_permit2_allowance(
        &self,
        token: Address,
        owner: Address,
    ) -> Pin<Box<dyn Future<Output = Result<U256, ClientError>> + Send + '_>> {
        Box::pin(async move {
            let (_addr, calldata) = permit2_allowance_calldata(token, owner);
            let tx = alloy_rpc_types_eth::TransactionRequest::default()
                .to(token)
                .input(calldata.into());
            let result = self.provider.call(tx).await.map_err(|e| {
                ClientError::PreConditionFailed(format!("Permit2 allowance check failed: {e}"))
            })?;
            Ok(U256::from_be_slice(&result))
        })
    }

    fn approve_permit2(
        &self,
        token: Address,
        _owner: Address,
    ) -> Pin<Box<dyn Future<Output = Result<(), ClientError>> + Send + '_>> {
        Box::pin(async move {
            let calldata = IPermit2Approval::approveCall {
                spender: PERMIT2_ADDRESS,
                amount: U256::MAX,
            }
            .abi_encode();
            let tx = alloy_rpc_types_eth::TransactionRequest::default()
                .to(token)
                .input(calldata.into());
            let pending = self.provider.send_transaction(tx).await.map_err(|e| {
                ClientError::PreConditionFailed(format!("Permit2 approve tx failed: {e}"))
            })?;
            let receipt = pending.get_receipt().await.map_err(|e| {
                ClientError::PreConditionFailed(format!("Permit2 approve receipt failed: {e}"))
            })?;
            if !receipt.status() {
                return Err(ClientError::PreConditionFailed(
                    "Permit2 approve transaction reverted".into(),
                ));
            }
            Ok(())
        })
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
                let requirements: types::v2::PaymentRequirements = v.as_concrete()?;
                let chain_reference = Eip155ChainReference::try_from(&requirements.network).ok()?;
                let candidate = PaymentCandidate {
                    chain_id: requirements.network.clone(),
                    asset: requirements.asset.to_string(),
                    amount: requirements.amount.0.to_string(),
                    scheme: self.scheme().to_string(),
                    pay_to: requirements.pay_to.to_string(),
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
}

struct V2PayloadSigner<S> {
    signer: S,
    resource_info: Option<ResourceInfo>,
    chain_reference: Eip155ChainReference,
    requirements: types::v2::PaymentRequirements,
    approver: Option<Arc<dyn Permit2Approver>>,
    auto_approve: bool,
}

impl<S> PaymentCandidateSigner for V2PayloadSigner<S>
where
    S: Sync + SignerLike,
{
    fn sign_payment(&self) -> r402::facilitator::BoxFuture<'_, Result<String, ClientError>> {
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
                let params = Eip3009SigningParams {
                    chain_id: self.chain_reference.inner(),
                    asset_address: self.requirements.asset.0,
                    pay_to: self.requirements.pay_to.into(),
                    amount: self.requirements.amount.into(),
                    max_timeout_seconds: self.requirements.max_timeout_seconds,
                    extra: self.requirements.extra.clone(),
                };
                let eip3009_payload = sign_erc3009_authorization(&self.signer, &params).await?;
                ExactPayload::Eip3009(eip3009_payload)
            };

            let payload = types::v2::PaymentPayload {
                x402_version: v2::V2,
                accepted: self.requirements.clone(),
                resource: self.resource_info.clone(),
                payload: exact_payload,
                extensions: None,
            };
            let json = serde_json::to_vec(&payload)?;
            let b64 = Base64Bytes::encode(&json);

            Ok(b64.to_string())
        })
    }
}

sol! {
    /// Minimal ERC-20 interface for client-side allowance checks and approvals.
    #[allow(missing_docs)]
    interface IPermit2Approval {
        function allowance(address owner, address spender) external view returns (uint256);
        function approve(address spender, uint256 amount) external returns (bool);
    }
}

/// Returns the ABI-encoded calldata for checking a token's Permit2 allowance.
///
/// The returned tuple `(token_address, calldata)` can be used with any EVM
/// provider's `eth_call` to check whether `owner` has approved the canonical
/// Permit2 contract to spend their tokens.
///
/// # Automatic Alternative
///
/// If you use [`Eip155ExactClient::builder`] with a [`Permit2Approver`],
/// allowance checks and approvals are handled automatically. This function
/// is primarily useful for custom [`Permit2Approver`] implementations.
///
/// Mirrors Go SDK's `GetPermit2AllowanceReadParams`.
#[must_use]
pub fn permit2_allowance_calldata(token: Address, owner: Address) -> (Address, Bytes) {
    let call = IPermit2Approval::allowanceCall {
        owner,
        spender: PERMIT2_ADDRESS,
    };
    (token, call.abi_encode().into())
}

/// Returns the ABI-encoded calldata for approving the canonical Permit2
/// contract to spend an unlimited amount of `token`.
///
/// The returned tuple `(token_address, calldata)` represents a transaction
/// the user must send (paying gas) before using the Permit2 payment flow.
///
/// # Automatic Alternative
///
/// If you use [`Eip155ExactClient::builder`] with a [`Permit2Approver`],
/// this approval is sent automatically when needed. This function is
/// primarily useful for custom [`Permit2Approver`] implementations.
///
/// Mirrors Go SDK's `CreatePermit2ApprovalTxData`.
#[must_use]
pub fn permit2_approval_calldata(token: Address) -> (Address, Bytes) {
    let call = IPermit2Approval::approveCall {
        spender: PERMIT2_ADDRESS,
        amount: U256::MAX,
    };
    (token, call.abi_encode().into())
}
