//! Client-side payment signing for the EIP-155 "exact" scheme.
//!
//! This module provides [`V1Eip155ExactClient`] and [`V2Eip155ExactClient`] for
//! signing ERC-3009 `transferWithAuthorization` payments on EVM chains.
//! Both share the core signing logic via [`sign_erc3009_authorization`].

use alloy_primitives::{Address, FixedBytes, Signature, U256};
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{SolStruct, eip712_domain};
use r402::chain::ChainId;
use r402::encoding::Base64Bytes;
use r402::proto::PaymentRequired;
use r402::proto::v1::X402Version1;
use r402::proto::v2::{self, ResourceInfo};
use r402::scheme::X402SchemeId;
use r402::scheme::client::{PaymentCandidate, PaymentCandidateSigner, X402Error, X402SchemeClient};
use r402::timestamp::UnixTimestamp;
use rand::RngExt;
use rand::rng;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::chain::Eip155ChainReference;
use crate::exact::types;
use crate::exact::{
    ExactEvmPayload, ExactEvmPayloadAuthorization, ExactScheme, PaymentRequirementsExtra,
    TransferWithAuthorization, V1Eip155Exact, V2Eip155Exact,
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
/// Used by both V1 and V2 EIP-155 exact scheme clients.
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
///
/// This is the shared signing logic used by both V1 and V2 EIP-155 exact scheme clients.
/// It constructs the EIP-712 domain, builds the authorization struct with appropriate
/// timing parameters, and signs the resulting hash.
///
/// # Errors
///
/// Returns [`X402Error`] if EIP-712 signing fails.
pub async fn sign_erc3009_authorization<S: SignerLike + Sync>(
    signer: &S,
    params: &Eip3009SigningParams,
) -> Result<ExactEvmPayload, X402Error> {
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

    let authorization = ExactEvmPayloadAuthorization {
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
        .map_err(|e| X402Error::SigningError(format!("{e:?}")))?;

    Ok(ExactEvmPayload {
        signature: signature.as_bytes().into(),
        authorization,
    })
}

/// Client for signing V1 EIP-155 exact scheme payments.
///
/// This client handles the creation and signing of ERC-3009 `transferWithAuthorization`
/// payments for EVM chains. It accepts payment requirements from servers and produces
/// signed payment payloads that can be verified and settled by facilitators.
#[derive(Debug)]
pub struct V1Eip155ExactClient<S> {
    signer: S,
}

impl<S> V1Eip155ExactClient<S> {
    /// Creates a new V1 EIP-155 exact scheme client with the given signer.
    pub const fn new(signer: S) -> Self {
        Self { signer }
    }
}

impl<S> X402SchemeId for V1Eip155ExactClient<S> {
    fn namespace(&self) -> &str {
        V1Eip155Exact.namespace()
    }

    fn scheme(&self) -> &str {
        V1Eip155Exact.scheme()
    }
}

impl<S> X402SchemeClient for V1Eip155ExactClient<S>
where
    S: SignerLike + Clone + Send + Sync + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        let payment_required = match payment_required {
            PaymentRequired::V1(payment_required) => payment_required,
            PaymentRequired::V2(_) => {
                return vec![];
            }
        };
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: types::v1::PaymentRequirements = v.as_concrete()?;
                let chain_id = ChainId::from_network_name(&requirements.network)?;
                let chain_reference = Eip155ChainReference::try_from(chain_id.clone()).ok()?;
                let candidate = PaymentCandidate {
                    chain_id,
                    asset: requirements.asset.to_string(),
                    amount: requirements.max_amount_required,
                    scheme: self.scheme().to_string(),
                    x402_version: self.x402_version(),
                    pay_to: requirements.pay_to.to_string(),
                    signer: Box::new(V1PayloadSigner {
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

struct V1PayloadSigner<S> {
    signer: S,
    chain_reference: Eip155ChainReference,
    requirements: types::v1::PaymentRequirements,
}

impl<S> PaymentCandidateSigner for V1PayloadSigner<S>
where
    S: SignerLike + Sync,
{
    fn sign_payment(&self) -> Pin<Box<dyn Future<Output = Result<String, X402Error>> + Send + '_>> {
        Box::pin(async move {
            let params = Eip3009SigningParams {
                chain_id: self.chain_reference.inner(),
                asset_address: self.requirements.asset,
                pay_to: self.requirements.pay_to,
                amount: self.requirements.max_amount_required,
                max_timeout_seconds: self.requirements.max_timeout_seconds,
                extra: self.requirements.extra.clone(),
            };

            let evm_payload = sign_erc3009_authorization(&self.signer, &params).await?;

            let payload = types::v1::PaymentPayload {
                x402_version: X402Version1,
                scheme: ExactScheme,
                network: self.requirements.network.clone(),
                payload: evm_payload,
            };
            let json = serde_json::to_vec(&payload)?;
            let b64 = Base64Bytes::encode(&json);

            Ok(b64.to_string())
        })
    }
}

/// Client for signing V2 EIP-155 exact scheme payments.
///
/// This client handles the creation and signing of ERC-3009 `transferWithAuthorization`
/// payments for EVM chains using the V2 protocol. Unlike V1, V2 uses CAIP-2 chain IDs
/// and embeds the accepted requirements directly in the payment payload.
#[derive(Debug)]
pub struct V2Eip155ExactClient<S> {
    signer: S,
}

impl<S> V2Eip155ExactClient<S> {
    /// Creates a new V2 EIP-155 exact scheme client with the given signer.
    pub const fn new(signer: S) -> Self {
        Self { signer }
    }
}

impl<S> X402SchemeId for V2Eip155ExactClient<S> {
    fn namespace(&self) -> &str {
        V2Eip155Exact.namespace()
    }

    fn scheme(&self) -> &str {
        V2Eip155Exact.scheme()
    }
}

impl<S> X402SchemeClient for V2Eip155ExactClient<S>
where
    S: SignerLike + Clone + Send + Sync + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        let payment_required = match payment_required {
            PaymentRequired::V2(payment_required) => payment_required,
            PaymentRequired::V1(_) => {
                return vec![];
            }
        };
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: types::v2::PaymentRequirements = v.as_concrete()?;
                let chain_reference = Eip155ChainReference::try_from(&requirements.network).ok()?;
                let candidate = PaymentCandidate {
                    chain_id: requirements.network.clone(),
                    asset: requirements.asset.to_string(),
                    amount: requirements.amount.into(),
                    scheme: self.scheme().to_string(),
                    x402_version: self.x402_version(),
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
    fn sign_payment(&self) -> Pin<Box<dyn Future<Output = Result<String, X402Error>> + Send + '_>> {
        Box::pin(async move {
            let params = Eip3009SigningParams {
                chain_id: self.chain_reference.inner(),
                asset_address: self.requirements.asset.0,
                pay_to: self.requirements.pay_to.into(),
                amount: self.requirements.amount.into(),
                max_timeout_seconds: self.requirements.max_timeout_seconds,
                extra: self.requirements.extra.clone(),
            };

            let evm_payload = sign_erc3009_authorization(&self.signer, &params).await?;

            let payload = types::v2::PaymentPayload {
                x402_version: v2::X402Version2,
                accepted: self.requirements.clone(),
                resource: self.resource_info.clone(),
                payload: evm_payload,
            };
            let json = serde_json::to_vec(&payload)?;
            let b64 = Base64Bytes::encode(&json);

            Ok(b64.to_string())
        })
    }
}
