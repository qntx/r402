//! Client signing for EVM `auth-capture`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_sol_types::{SolStruct, eip712_domain, sol};
use r402_core::error::ClientError;
use r402_core::scheme::{
    PaymentCandidate, PaymentCandidateSigner, SchemeClient, SchemeId, sealed::Sealed,
};
use r402_core::wire::{Base64Bytes, PaymentRequired, UnixTimestamp};
use rand::RngExt;
use rand::rng;

use super::nonce::{compute_payer_agnostic_payment_info_hash, hash_as_uint256};
use super::types::AuthCaptureTokenPermissions;
use super::types::{
    AuthCaptureEip3009Authorization, AuthCaptureEip3009Payload, AuthCaptureExtra,
    AuthCapturePayload, AuthCapturePermit2Authorization, AuthCapturePermit2Payload,
    EIP3009_TOKEN_COLLECTOR_ADDRESS, PERMIT2_TOKEN_COLLECTOR_ADDRESS, PaymentInfo,
};
use crate::chain::TokenAmount;
use crate::exact::client::SignerLike;
use crate::exact::{AssetTransferMethod, PERMIT2_ADDRESS};

sol! {
    struct ReceiveWithAuthorization {
        address from;
        address to;
        uint256 value;
        uint256 validAfter;
        uint256 validBefore;
        bytes32 nonce;
    }

    struct TokenPermissions {
        address token;
        uint256 amount;
    }

    struct PermitTransferFrom {
        TokenPermissions permitted;
        address spender;
        uint256 nonce;
        uint256 deadline;
    }
}

/// Client for signing auth-capture payments.
pub struct Eip155AuthCaptureClient<S> {
    signer: Arc<S>,
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
    pub fn new(signer: S) -> Self {
        Self {
            signer: Arc::new(signer),
        }
    }

    /// Builds from an existing `Arc`.
    #[must_use]
    pub const fn from_arc(signer: Arc<S>) -> Self {
        Self { signer }
    }
}

impl<S: SignerLike + Sync + 'static> SchemeId for Eip155AuthCaptureClient<S> {
    fn namespace(&self) -> &'static str {
        "eip155"
    }

    fn scheme(&self) -> &'static str {
        "auth-capture"
    }
}

impl<S: SignerLike + Sync + 'static> Sealed for Eip155AuthCaptureClient<S> {}

impl<S: SignerLike + Sync + 'static> SchemeClient for Eip155AuthCaptureClient<S> {
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        let mut out = Vec::new();
        for req in &payment_required.accepts {
            if req.scheme.as_str() != "auth-capture" {
                continue;
            }
            if !req.network.namespace().eq_ignore_ascii_case("eip155") {
                continue;
            }
            let Some(extra_val) = req.extra.clone() else {
                continue;
            };
            let Ok(extra) = serde_json::from_value::<AuthCaptureExtra>(extra_val) else {
                continue;
            };
            let Ok(amount) = req.amount.parse::<U256>() else {
                continue;
            };
            let Ok(pay_to) = req.pay_to.parse::<Address>() else {
                continue;
            };
            let Ok(asset) = req.asset.parse::<Address>() else {
                continue;
            };
            let Ok(chain_id) = req.network.reference().parse::<u64>() else {
                continue;
            };
            out.push(PaymentCandidate {
                chain_id: req.network.clone(),
                asset: req.asset.clone(),
                amount: req.amount.clone(),
                scheme: "auth-capture".into(),
                pay_to: req.pay_to.clone(),
                signer: Box::new(AuthCaptureCandidateSigner {
                    signer: Arc::clone(&self.signer),
                    chain_id,
                    asset,
                    pay_to,
                    amount,
                    max_timeout: req.max_timeout_seconds,
                    extra,
                }),
            });
        }
        out
    }
}

struct AuthCaptureCandidateSigner<S> {
    signer: Arc<S>,
    chain_id: u64,
    asset: Address,
    pay_to: Address,
    amount: U256,
    max_timeout: u64,
    extra: AuthCaptureExtra,
}

impl<S: SignerLike + Sync + 'static> PaymentCandidateSigner for AuthCaptureCandidateSigner<S> {
    fn sign_payment<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + 'a>> {
        Box::pin(async move {
            let payload = sign_auth_capture(
                self.signer.as_ref(),
                self.chain_id,
                self.asset,
                self.pay_to,
                self.amount,
                self.max_timeout,
                &self.extra,
            )
            .await?;
            let json = serde_json::to_vec(&payload).map_err(ClientError::Json)?;
            Ok(Base64Bytes::encode(json).to_string())
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
    let nonce = compute_payer_agnostic_payment_info_hash(chain_id, &payment_info);

    match extra.transfer_method() {
        AssetTransferMethod::Permit2 => {
            sign_permit2_auth_capture(
                signer,
                chain_id,
                asset,
                amount,
                pre_approval_expiry,
                nonce,
                salt,
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
    } = params;
    let domain = eip712_domain! {
        name: extra.name.clone(),
        version: extra.version.clone(),
        chain_id: chain_id,
        verifying_contract: asset,
    };
    let authorization = AuthCaptureEip3009Authorization {
        from: signer.address(),
        to: EIP3009_TOKEN_COLLECTOR_ADDRESS,
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
    }))
}

async fn sign_permit2_auth_capture<S: SignerLike + Sync>(
    signer: &S,
    chain_id: u64,
    asset: Address,
    amount: U256,
    pre_approval_expiry: u64,
    nonce: B256,
    salt: B256,
) -> Result<AuthCapturePayload, ClientError> {
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
        spender: PERMIT2_TOKEN_COLLECTOR_ADDRESS,
        nonce: TokenAmount::from(nonce_u256),
        deadline: TokenAmount::from(U256::from(pre_approval_expiry)),
    };
    let typed = PermitTransferFrom {
        permitted: TokenPermissions {
            token: asset,
            amount,
        },
        spender: PERMIT2_TOKEN_COLLECTOR_ADDRESS,
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
    }))
}
