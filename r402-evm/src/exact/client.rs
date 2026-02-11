//! Client-side payment signing for the EIP-155 "exact" scheme.
//!
//! This module provides [`Eip155ExactClient`] for signing ERC-3009
//! `transferWithAuthorization` payments on EVM chains.

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
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

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
/// This client handles the creation and signing of ERC-3009 `transferWithAuthorization`
/// payments for EVM chains. Uses CAIP-2 chain IDs and embeds the accepted requirements
/// directly in the payment payload.
#[derive(Debug)]
pub struct Eip155ExactClient<S> {
    signer: S,
}

impl<S> Eip155ExactClient<S> {
    /// Creates a new EIP-155 exact scheme client with the given signer.
    pub const fn new(signer: S) -> Self {
        Self { signer }
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
}

impl<S> PaymentCandidateSigner for V2PayloadSigner<S>
where
    S: Sync + SignerLike,
{
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
/// Mirrors Go SDK's `CreatePermit2ApprovalTxData`.
#[must_use]
pub fn permit2_approval_calldata(token: Address) -> (Address, Bytes) {
    let call = IPermit2Approval::approveCall {
        spender: PERMIT2_ADDRESS,
        amount: U256::MAX,
    };
    (token, call.abi_encode().into())
}
