//! Client-side payment signing for the Tron "exact" scheme.
//!
//! This module provides [`TronExactClient`] for signing Tron payments,
//! supporting both EIP-3009 (`transferWithAuthorization`) and SUN.io
//! Permit2 transfer methods. The transfer method is selected automatically
//! based on the server's `PaymentRequirements.extra.assetTransferMethod`.
//!
//! Unlike `r402-evm`, there is no Permit2 auto-approve support: Tron has no
//! general-purpose provider/transaction-sending abstraction in this crate,
//! so callers must ensure the payer has already approved the SUN.io Permit2
//! contract (see [`crate::chain::TronChainReference::sun_permit2`]) before
//! signing a Permit2 payment.

use std::future::Future;
use std::sync::Arc;

use alloy_primitives::{Address as EvmAddress, FixedBytes, Signature, U256};
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{SolStruct, eip712_domain};
use r402_core::chain::ChainId;
use r402_core::error::ClientError;
use r402_core::scheme::SchemeId;
use r402_core::scheme::{DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use r402_core::wire::{Base64Bytes, PaymentRequired, ResourceInfo, UnixTimestamp};
use rand::RngExt;
use rand::rng;

use crate::chain::TronChainReference;
use crate::exact::types;
use crate::exact::{
    AssetTransferMethod, Eip3009Authorization, Eip3009Payload, ExactPayload,
    PaymentRequirementsExtra, Permit2Authorization, Permit2Payload, Permit2TokenPermissions,
    Permit2Witness, TransferWithAuthorization, TronExact, TronPermitWitnessTransferFrom,
    TronTokenPermissions, TronWitness,
};

/// Abstraction over signing operations, allowing both owned and `Arc`-wrapped signers.
///
/// Tron reuses secp256k1 ECDSA key material identical to Ethereum, so any
/// `alloy_signer::Signer` (e.g. [`PrivateKeySigner`]) works as a Tron
/// signer — only the *derived wire address encoding* differs
/// (`Base58Check` vs. hex), and that conversion happens above this layer.
pub trait SignerLike: Send + Sync {
    /// Returns the raw EVM-form address of the signer.
    fn address(&self) -> EvmAddress;

    /// Signs the given TIP-712 signing hash.
    fn sign_hash(
        &self,
        hash: &FixedBytes<32>,
    ) -> impl Future<Output = Result<Signature, alloy_signer::Error>> + Send;
}

impl SignerLike for PrivateKeySigner {
    fn address(&self) -> EvmAddress {
        alloy_signer::Signer::address(self)
    }

    async fn sign_hash(&self, hash: &FixedBytes<32>) -> Result<Signature, alloy_signer::Error> {
        alloy_signer::Signer::sign_hash(self, hash).await
    }
}

impl<T: SignerLike + Send + Sync> SignerLike for Arc<T> {
    fn address(&self) -> EvmAddress {
        (**self).address()
    }

    async fn sign_hash(&self, hash: &FixedBytes<32>) -> Result<Signature, alloy_signer::Error> {
        (**self).sign_hash(hash).await
    }
}

/// Shared TIP-712 signing parameters for EIP-3009 authorization.
#[derive(Debug, Clone)]
pub struct Eip3009SigningParams {
    /// The Tron chain reference (used as the numeric TIP-712 domain `chainId`).
    pub chain_reference: TronChainReference,
    /// The token contract address (verifying contract for TIP-712), raw EVM hex form.
    pub asset_address: EvmAddress,
    /// The recipient address for the transfer, raw EVM hex form.
    pub pay_to: EvmAddress,
    /// The amount to transfer.
    pub amount: U256,
    /// Maximum timeout in seconds for the authorization validity window.
    pub max_timeout_seconds: u64,
    /// TIP-712 domain `name` and `version` for the token. Required by
    /// EIP-3009; must match what the token contract declares.
    pub extra: PaymentRequirementsExtra,
}

/// Signs an EIP-3009 `TransferWithAuthorization` using TIP-712.
///
/// # Errors
///
/// Returns [`ClientError`] if the domain parameters are missing or signing fails.
pub async fn sign_eip3009_authorization<S: SignerLike + Sync>(
    signer: &S,
    params: &Eip3009SigningParams,
) -> Result<Eip3009Payload, ClientError> {
    if params.extra.name.is_empty() || params.extra.version.is_empty() {
        return Err(ClientError::Signing(
            "EIP-3009 TIP-712 domain requires non-empty `name` and `version`".into(),
        ));
    }

    let domain = eip712_domain! {
        name: params.extra.name.clone(),
        version: params.extra.version.clone(),
        chain_id: u64::from(params.chain_reference.inner()),
        verifying_contract: params.asset_address,
    };

    let now = UnixTimestamp::now();
    let valid_after_secs = now.as_secs().saturating_sub(10 * 60);
    let valid_after = UnixTimestamp::from_secs(valid_after_secs);
    let valid_before = now + params.max_timeout_seconds;
    let nonce = FixedBytes(rng().random::<[u8; 32]>());

    let authorization = Eip3009Authorization {
        from: signer.address(),
        to: params.pay_to,
        value: params.amount.into(),
        valid_after,
        valid_before,
        nonce,
    };

    let transfer_with_authorization = TransferWithAuthorization {
        from: authorization.from,
        to: authorization.to,
        value: authorization.value.into(),
        validAfter: U256::from(authorization.valid_after.as_secs()),
        validBefore: U256::from(authorization.valid_before.as_secs()),
        nonce: authorization.nonce,
    };

    let signing_hash = transfer_with_authorization.eip712_signing_hash(&domain);
    let signature = signer
        .sign_hash(&signing_hash)
        .await
        .map_err(|e| ClientError::Signing(format!("{e:?}")))?;

    Ok(Eip3009Payload {
        signature: signature.as_bytes().into(),
        authorization,
    })
}

/// Shared signing parameters for SUN.io Permit2 authorization.
#[derive(Debug, Clone, Copy)]
pub struct Permit2SigningParams {
    /// The Tron chain reference (used as the numeric TIP-712 domain `chainId`).
    pub chain_reference: TronChainReference,
    /// The token contract address, raw EVM hex form.
    pub asset_address: EvmAddress,
    /// The recipient address for the transfer, raw EVM hex form.
    pub pay_to: EvmAddress,
    /// The amount to transfer.
    pub amount: U256,
    /// Maximum timeout in seconds for the authorization validity window.
    pub max_timeout_seconds: u64,
}

/// Signs a SUN.io Permit2 `PermitWitnessTransferFrom` using TIP-712.
///
/// # Errors
///
/// Returns [`ClientError::Signing`] if the network has no known SUN.io
/// Permit2 / `x402ExactPermit2Proxy` deployment, or if signing fails.
pub async fn sign_permit2_authorization<S: SignerLike + Sync>(
    signer: &S,
    params: &Permit2SigningParams,
) -> Result<Permit2Payload, ClientError> {
    let permit2 = params.chain_reference.sun_permit2().ok_or_else(|| {
        ClientError::Signing(format!(
            "no known SUN.io Permit2 deployment for chain {}",
            params.chain_reference
        ))
    })?;
    let proxy = params
        .chain_reference
        .x402_exact_permit2_proxy()
        .ok_or_else(|| {
            ClientError::Signing(format!(
                "no known x402ExactPermit2Proxy deployment for chain {}",
                params.chain_reference
            ))
        })?;

    let domain = eip712_domain! {
        name: "Permit2",
        chain_id: u64::from(params.chain_reference.inner()),
        verifying_contract: permit2.as_evm(),
    };

    let now = UnixTimestamp::now();
    let valid_after_secs = now.as_secs().saturating_sub(10 * 60);
    let deadline_secs = now.as_secs() + params.max_timeout_seconds;
    let nonce = U256::from_be_bytes(rng().random::<[u8; 32]>());

    let permit_witness = TronPermitWitnessTransferFrom {
        permitted: TronTokenPermissions {
            token: params.asset_address,
            amount: params.amount,
        },
        spender: proxy.as_evm(),
        nonce,
        deadline: U256::from(deadline_secs),
        witness: TronWitness {
            to: params.pay_to,
            validAfter: U256::from(valid_after_secs),
        },
    };

    let signing_hash = permit_witness.eip712_signing_hash(&domain);
    let signature = signer
        .sign_hash(&signing_hash)
        .await
        .map_err(|e| ClientError::Signing(format!("{e:?}")))?;

    let authorization = Permit2Authorization {
        from: signer.address(),
        permitted: Permit2TokenPermissions {
            token: params.asset_address,
            amount: params.amount.into(),
        },
        spender: proxy.as_evm(),
        nonce: nonce.into(),
        deadline: U256::from(deadline_secs).into(),
        witness: Permit2Witness {
            to: params.pay_to,
            valid_after: U256::from(valid_after_secs).into(),
        },
    };

    Ok(Permit2Payload {
        signature: signature.as_bytes().into(),
        permit2_authorization: authorization,
    })
}

/// Client for signing Tron exact scheme payments.
///
/// Supports both EIP-3009 (`transferWithAuthorization`) and SUN.io Permit2
/// transfer methods, selected automatically based on the server's
/// `PaymentRequirements.extra.assetTransferMethod`.
#[derive(Clone)]
pub struct TronExactClient<S> {
    signer: S,
}

impl<S: std::fmt::Debug> std::fmt::Debug for TronExactClient<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TronExactClient")
            .field("signer", &self.signer)
            .finish()
    }
}

impl<S> TronExactClient<S> {
    /// Creates a new Tron exact scheme client with the given signer.
    pub const fn new(signer: S) -> Self {
        Self { signer }
    }
}

impl<S> SchemeId for TronExactClient<S> {
    fn namespace(&self) -> &str {
        TronExact.namespace()
    }

    fn scheme(&self) -> &str {
        TronExact.scheme()
    }
}

impl<S> r402_core::scheme::Sealed for TronExactClient<S> {}

impl<S> SchemeClient for TronExactClient<S>
where
    S: SignerLike + Clone + Send + Sync + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: types::v2::PaymentRequirements = v.as_concrete()?;
                let chain_reference =
                    TronChainReference::try_from(requirements.network.clone()).ok()?;
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
                    }),
                };
                Some(candidate)
            })
            .collect::<Vec<_>>()
    }

    fn find_default_asset(&self, asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
        crate::find_default_tron_asset(asset, network)
    }
}

struct V2PayloadSigner<S> {
    signer: S,
    resource_info: Option<ResourceInfo>,
    chain_reference: TronChainReference,
    requirements: types::v2::PaymentRequirements,
}

impl<S> PaymentCandidateSigner for V2PayloadSigner<S>
where
    S: Sync + SignerLike,
{
    fn sign_payment(&self) -> r402_core::facilitator::BoxFuture<'_, Result<String, ClientError>> {
        Box::pin(async move {
            let use_permit2 = self
                .requirements
                .extra
                .as_ref()
                .and_then(|e| e.asset_transfer_method)
                == Some(AssetTransferMethod::Permit2);

            let exact_payload = if use_permit2 {
                let params = Permit2SigningParams {
                    chain_reference: self.chain_reference,
                    asset_address: self.requirements.asset.as_evm(),
                    pay_to: self.requirements.pay_to.as_evm(),
                    amount: self.requirements.amount.into(),
                    max_timeout_seconds: self.requirements.max_timeout_seconds,
                };
                let permit2_payload = sign_permit2_authorization(&self.signer, &params).await?;
                ExactPayload::Permit2(permit2_payload)
            } else {
                const ERR_MSG: &str = "EIP-3009 payment requires `paymentRequirements.extra` \
                         with the token's TIP-712 domain `name` and `version`";
                let extra = self
                    .requirements
                    .extra
                    .clone()
                    .ok_or_else(|| ClientError::Signing(ERR_MSG.into()))?;
                let params = Eip3009SigningParams {
                    chain_reference: self.chain_reference,
                    asset_address: self.requirements.asset.as_evm(),
                    pay_to: self.requirements.pay_to.as_evm(),
                    amount: self.requirements.amount.into(),
                    max_timeout_seconds: self.requirements.max_timeout_seconds,
                    extra,
                };
                let eip3009_payload = sign_eip3009_authorization(&self.signer, &params).await?;
                ExactPayload::Eip3009(eip3009_payload)
            };

            let payload = types::v2::PaymentPayload::new(self.requirements.clone(), exact_payload)
                .with_optional_resource(self.resource_info.clone());
            let json = serde_json::to_vec(&payload)?;
            let b64 = Base64Bytes::encode(&json);

            Ok(b64.to_string())
        })
    }
}
