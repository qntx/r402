//! Client signing for EVM `auth-capture`.

use std::future::Future;
use std::pin::Pin;

use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_sol_types::{SolStruct, eip712_domain};
use r402_client::{DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use r402_protocol::error::ClientError;
use r402_protocol::network::ChainId;
use r402_protocol::payment::{Base64Bytes, PaymentRequired, ResourceInfo, UnixTimestamp};
use r402_protocol::scheme::SchemeId;
use rand::{RngExt, rng};

use super::nonce::{compute_payer_agnostic_payment_info_hash, hash_as_uint256};
use super::payload::{
    AuthCaptureEip3009Authorization, AuthCaptureEip3009Payload, AuthCaptureExtra,
    AuthCapturePayload, AuthCapturePermit2Authorization, AuthCapturePermit2Payload,
    AuthCaptureTokenPermissions, PaymentInfo, PermitTransferFrom, ReceiveWithAuthorization,
    TokenPermissions,
};
use super::{Eip155AuthCapture, payload};
use crate::asset::AssetTransferMethod;
use crate::chain::{Eip155ChainReference, TokenAmount};
use crate::permit2::PERMIT2_ADDRESS;
use crate::signer::SignerLike;

/// Client for signing auth-capture payments.
pub struct Eip155AuthCaptureClient<S> {
    signer: S,
}

impl<S: std::fmt::Debug> std::fmt::Debug for Eip155AuthCaptureClient<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Eip155AuthCaptureClient")
            .finish_non_exhaustive()
    }
}

impl<S> Eip155AuthCaptureClient<S> {
    /// Builds a client around `signer`.
    #[must_use]
    pub const fn new(signer: S) -> Self {
        Self { signer }
    }
}

impl<S> SchemeId for Eip155AuthCaptureClient<S> {
    fn namespace(&self) -> &'static str {
        Eip155AuthCapture.namespace()
    }

    fn scheme(&self) -> &str {
        Eip155AuthCapture.scheme()
    }
}

impl<S> SchemeClient for Eip155AuthCaptureClient<S>
where
    S: SignerLike + Clone + Send + Sync + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: payload::v2::PaymentRequirements = v.as_concrete()?;
                let extra = requirements.extra.clone()?;
                let chain_reference = Eip155ChainReference::try_from(&requirements.network).ok()?;
                Some(PaymentCandidate {
                    chain_id: requirements.network.clone(),
                    asset: requirements.asset.to_string().into(),
                    amount: requirements.amount.0.to_string().into(),
                    scheme: self.scheme().into(),
                    pay_to: requirements.pay_to.to_string().into(),
                    requirements: v.clone(),
                    signer: Box::new(AuthCaptureCandidateSigner {
                        signer: self.signer.clone(),
                        resource_info: Some(payment_required.resource.clone()),
                        chain_id: chain_reference.inner(),
                        requirements,
                        extra,
                    }),
                })
            })
            .collect()
    }

    fn find_default_asset(&self, asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
        crate::find_default_evm_asset_info(asset, network)
    }
}

struct AuthCaptureCandidateSigner<S> {
    signer: S,
    resource_info: Option<ResourceInfo>,
    chain_id: u64,
    requirements: payload::v2::PaymentRequirements,
    extra: AuthCaptureExtra,
}

impl<S> PaymentCandidateSigner for AuthCaptureCandidateSigner<S>
where
    S: SignerLike + Sync,
{
    fn sign_payment<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + 'a>> {
        Box::pin(async move {
            let scheme_payload = sign_auth_capture(
                &self.signer,
                self.chain_id,
                self.requirements.asset.0,
                self.requirements.pay_to.0,
                self.requirements.amount.0,
                self.requirements.max_timeout_seconds,
                &self.extra,
            )
            .await?;
            let payload =
                payload::v2::PaymentPayload::new(self.requirements.clone(), scheme_payload)
                    .with_optional_resource(self.resource_info.clone());
            let json = serde_json::to_vec(&payload)?;
            Ok(Base64Bytes::encode(&json).to_string())
        })
    }
}

/// Signs an auth-capture payload (EIP-3009 or Permit2 per `extra`).
///
/// # Errors
///
/// Missing domain fields or signing failure.
pub async fn sign_auth_capture<S: SignerLike + Sync>(
    signer: &S,
    chain_id: u64,
    asset: Address,
    pay_to: Address,
    amount: U256,
    max_timeout_seconds: u64,
    extra: &AuthCaptureExtra,
) -> Result<AuthCapturePayload, ClientError> {
    if extra.name.is_empty() || extra.version.is_empty() {
        return Err(ClientError::Signing(
            "auth-capture extra.name and extra.version are required".into(),
        ));
    }
    let deployment = extra.deployment().ok_or_else(|| {
        ClientError::Signing("Invalid authCaptureEscrow in payment requirements extra".into())
    })?;
    let now = UnixTimestamp::now().as_secs();
    let pre_approval_expiry = now.saturating_add(max_timeout_seconds);
    let salt_bytes: [u8; 32] = rng().random();
    let salt = B256::from(salt_bytes);
    let salt_u256 = U256::from_be_bytes(salt.0);

    let payment_info = PaymentInfo {
        operator: extra.capture_authorizer.0,
        payer: signer.address(),
        receiver: pay_to,
        token: asset,
        max_amount: amount,
        pre_approval_expiry,
        authorization_expiry: extra.capture_deadline,
        refund_expiry: extra.refund_deadline,
        min_fee_bps: extra.min_fee_bps,
        max_fee_bps: extra.max_fee_bps,
        fee_receiver: extra.fee_recipient.0,
        salt: salt_u256,
    };
    let nonce =
        compute_payer_agnostic_payment_info_hash(chain_id, &payment_info, deployment.escrow);

    match extra.transfer_method() {
        AssetTransferMethod::Permit2 => {
            sign_permit2_auth_capture(
                signer,
                Permit2SignParams {
                    chain_id,
                    asset,
                    amount,
                    pre_approval_expiry,
                    nonce,
                    salt,
                    spender: deployment.permit2_collector,
                },
            )
            .await
        }
        AssetTransferMethod::Eip3009 => {
            sign_eip3009_auth_capture(
                signer,
                Eip3009SignParams {
                    chain_id,
                    asset,
                    amount,
                    pre_approval_expiry,
                    nonce,
                    salt,
                    to: deployment.eip3009_collector,
                },
                extra,
            )
            .await
        }
    }
}

/// Inputs for an EIP-3009 auth-capture signature (keeps the signer arity low).
struct Eip3009SignParams {
    chain_id: u64,
    asset: Address,
    amount: U256,
    pre_approval_expiry: u64,
    nonce: B256,
    salt: B256,
    to: Address,
}

/// Inputs for a Permit2 auth-capture signature.
struct Permit2SignParams {
    chain_id: u64,
    asset: Address,
    amount: U256,
    pre_approval_expiry: u64,
    nonce: B256,
    salt: B256,
    spender: Address,
}

async fn sign_eip3009_auth_capture<S: SignerLike + Sync>(
    signer: &S,
    params: Eip3009SignParams,
    extra: &AuthCaptureExtra,
) -> Result<AuthCapturePayload, ClientError> {
    let Eip3009SignParams {
        chain_id,
        asset,
        amount,
        pre_approval_expiry,
        nonce,
        salt,
        to,
    } = params;
    let domain = eip712_domain! {
        name: extra.name.clone(),
        version: extra.version.clone(),
        chain_id: chain_id,
        verifying_contract: asset,
    };
    let authorization = AuthCaptureEip3009Authorization {
        from: signer.address(),
        to,
        value: TokenAmount::from(amount),
        valid_after: UnixTimestamp::from_secs(0),
        valid_before: UnixTimestamp::from_secs(pre_approval_expiry),
        nonce,
    };
    let typed = ReceiveWithAuthorization {
        from: authorization.from,
        to: authorization.to,
        value: amount,
        validAfter: U256::ZERO,
        validBefore: U256::from(pre_approval_expiry),
        nonce,
    };
    let hash = typed.eip712_signing_hash(&domain);
    let signature = signer
        .sign_hash(&hash)
        .await
        .map_err(|e| ClientError::Signing(format!("{e:?}")))?;
    Ok(AuthCapturePayload::Eip3009(AuthCaptureEip3009Payload {
        authorization,
        signature: Bytes::from(signature.as_bytes().to_vec()),
        salt,
        fee_bps: None,
        fee_amount: None,
    }))
}

async fn sign_permit2_auth_capture<S: SignerLike + Sync>(
    signer: &S,
    params: Permit2SignParams,
) -> Result<AuthCapturePayload, ClientError> {
    let Permit2SignParams {
        chain_id,
        asset,
        amount,
        pre_approval_expiry,
        nonce,
        salt,
        spender,
    } = params;
    let domain = eip712_domain! {
        name: "Permit2",
        chain_id: chain_id,
        verifying_contract: PERMIT2_ADDRESS,
    };
    let nonce_u256 = hash_as_uint256(nonce);
    let permit2_authorization = AuthCapturePermit2Authorization {
        from: signer.address(),
        permitted: AuthCaptureTokenPermissions {
            token: asset,
            amount: TokenAmount::from(amount),
        },
        spender,
        nonce: TokenAmount::from(nonce_u256),
        deadline: TokenAmount::from(U256::from(pre_approval_expiry)),
    };
    let typed = PermitTransferFrom {
        permitted: TokenPermissions {
            token: asset,
            amount,
        },
        spender,
        nonce: nonce_u256,
        deadline: U256::from(pre_approval_expiry),
    };
    let hash = typed.eip712_signing_hash(&domain);
    let signature = signer
        .sign_hash(&hash)
        .await
        .map_err(|e| ClientError::Signing(format!("{e:?}")))?;
    Ok(AuthCapturePayload::Permit2(AuthCapturePermit2Payload {
        permit2_authorization,
        signature: Bytes::from(signature.as_bytes().to_vec()),
        salt,
        fee_bps: None,
        fee_amount: None,
    }))
}
