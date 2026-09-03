//! Client-side payment-channel `open` for the Solana `upto` scheme.

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use r402_client::{DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use r402_protocol::scheme::SchemeId;
use r402_protocol::{Base64Bytes, ChainId, ClientError, PaymentRequired, ResourceInfo};
use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_message::Hash;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

use super::SolanaUpto;
use super::channel::{BuildOpenArgs, build_open_transaction};
use super::channel::{
    parse_extra_memo, parse_extra_u64, parse_token_program_hint, resolve_payment_channel_config,
};
use super::payload::{UptoSvmPayload, extra_keys, v2};
use crate::chain::rpc::RpcClientLike;
use crate::exact::client::fetch_mint;
use crate::exact::payload::TransactionInt;

/// Solana upto scheme client: signs a channel `open` for the authorized max.
#[derive(Clone)]
pub struct SolanaUptoClient<S, R> {
    signer: S,
    rpc_client: R,
}

impl<S, R> std::fmt::Debug for SolanaUptoClient<S, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SolanaUptoClient").finish_non_exhaustive()
    }
}

impl<S, R> SolanaUptoClient<S, R> {
    /// Creates a new Solana upto client.
    pub const fn new(signer: S, rpc_client: R) -> Self {
        Self { signer, rpc_client }
    }
}

impl<S, R> SchemeId for SolanaUptoClient<S, R> {
    fn namespace(&self) -> &str {
        SolanaUpto.namespace()
    }

    fn scheme(&self) -> &str {
        SolanaUpto.scheme()
    }
}

impl<S, R> SchemeClient for SolanaUptoClient<S, R>
where
    S: Signer + Send + Sync + Clone + 'static,
    R: RpcClientLike + Send + Sync + Clone + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: v2::PaymentRequirements = v.as_concrete()?;
                let chain_id = requirements.network.clone();
                if chain_id.namespace() != "solana" {
                    return None;
                }
                Some(PaymentCandidate {
                    chain_id,
                    asset: requirements.asset.to_string().into(),
                    amount: requirements.amount.inner().to_string().into(),
                    scheme: self.scheme().into(),
                    pay_to: requirements.pay_to.to_string().into(),
                    requirements: v.clone(),
                    signer: Box::new(UptoPayloadSigner {
                        signer: self.signer.clone(),
                        rpc_client: self.rpc_client.clone(),
                        requirements,
                        resource: payment_required.resource.clone(),
                    }),
                })
            })
            .collect()
    }

    fn find_default_asset(&self, asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
        crate::find_default_solana_asset(asset, network)
    }
}

struct UptoPayloadSigner<S, R> {
    signer: S,
    rpc_client: R,
    requirements: v2::PaymentRequirements,
    resource: ResourceInfo,
}

impl<S: Signer + Send + Sync, R: RpcClientLike + Send + Sync> PaymentCandidateSigner
    for UptoPayloadSigner<S, R>
{
    fn sign_payment(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + '_>> {
        Box::pin(async move {
            let payload =
                build_upto_payload(&self.signer, &self.rpc_client, &self.requirements).await?;
            let wire = v2::PaymentPayload::new(self.requirements.clone(), payload)
                .with_resource(self.resource.clone());
            let json = serde_json::to_vec(&wire)?;
            Ok(Base64Bytes::encode(&json).to_string())
        })
    }
}

async fn build_upto_payload<S: Signer + Sync, R: RpcClientLike>(
    signer: &S,
    rpc: &R,
    requirements: &v2::PaymentRequirements,
) -> Result<UptoSvmPayload, ClientError> {
    let untyped = untyped_requirements(requirements);
    let channel = resolve_payment_channel_config(&untyped).map_err(sign_err)?;
    let fee_payer: Pubkey = channel.fee_payer.parse().map_err(sign_err)?;
    let authorized: Pubkey = channel.receiver_authorizer.parse().map_err(sign_err)?;
    let mint = *requirements.asset.pubkey();
    let max_amount = requirements.amount.inner();
    let token_program = resolve_client_token_program(rpc, requirements).await?;
    let blockhash = resolve_blockhash(rpc, requirements.extra.as_ref()).await?;
    let open_slot = resolve_open_slot(rpc, requirements.extra.as_ref()).await?;
    let built = build_open_transaction(BuildOpenArgs {
        payer: signer.pubkey(),
        payee: fee_payer,
        mint,
        authorized_signer: authorized,
        fee_payer,
        token_program,
        deposit: max_amount,
        blockhash,
        open_slot,
        grace_period: channel.withdraw_delay,
        recipients: channel.splits,
        salt: None,
        memo: parse_extra_memo(requirements.extra.as_ref()),
    })
    .map_err(sign_err)?;
    let signed = TransactionInt::new(built.transaction)
        .sign_with_keypair(signer)
        .map_err(|err| ClientError::Signing(err.to_string()))?;
    let open_transaction = signed
        .as_base64()
        .map_err(|err| ClientError::Signing(err.to_string()))?;
    let now = unix_now()?;
    let valid_after = parse_extra_u64(
        requirements
            .extra
            .as_ref()
            .and_then(|extra| extra.get(extra_keys::VALID_AFTER)),
    )
    .and_then(|v| i64::try_from(v).ok())
    .unwrap_or(now);
    Ok(UptoSvmPayload {
        from: signer.pubkey().to_string(),
        max_amount: max_amount.to_string(),
        expires_at: now
            .saturating_add(i64::try_from(requirements.max_timeout_seconds).unwrap_or(i64::MAX)),
        valid_after,
        nonce: built.salt.to_string(),
        open_slot: built.open_slot.to_string(),
        channel_id: built.channel_id.to_string(),
        deposit: max_amount.to_string(),
        authorized_signer: channel.receiver_authorizer,
        open_transaction,
        voucher_signature: None,
    })
}

async fn resolve_client_token_program<R: RpcClientLike>(
    rpc: &R,
    requirements: &v2::PaymentRequirements,
) -> Result<Pubkey, ClientError> {
    if let Some(hint) = parse_token_program_hint(requirements.extra.as_ref()).map_err(sign_err)? {
        return Ok(hint);
    }
    let mint = fetch_mint(&requirements.asset, rpc).await?;
    Ok(*mint.token_program())
}

async fn resolve_blockhash<R: RpcClientLike>(
    rpc: &R,
    extra: Option<&serde_json::Value>,
) -> Result<Hash, ClientError> {
    if let Some(hint) = extra
        .and_then(|v| v.get(extra_keys::RECENT_BLOCKHASH))
        .and_then(serde_json::Value::as_str)
        && let Ok(hash) = Hash::from_str(hint)
    {
        return Ok(hash);
    }
    let (hash, _) = rpc
        .get_latest_blockhash_with_commitment(CommitmentConfig {
            commitment: CommitmentLevel::Finalized,
        })
        .await
        .map_err(|err| ClientError::Signing(err.to_string()))?;
    Ok(hash)
}

async fn resolve_open_slot<R: RpcClientLike>(
    rpc: &R,
    extra: Option<&serde_json::Value>,
) -> Result<u64, ClientError> {
    if let Some(slot) = parse_extra_u64(extra.and_then(|v| v.get(extra_keys::RECENT_SLOT))) {
        return Ok(slot);
    }
    rpc.get_slot_with_commitment(CommitmentConfig {
        commitment: CommitmentLevel::Finalized,
    })
    .await
    .map_err(|err| ClientError::Signing(err.to_string()))
}

fn untyped_requirements(
    requirements: &v2::PaymentRequirements,
) -> r402_protocol::PaymentRequirements {
    r402_protocol::PaymentRequirements::new(
        requirements.scheme.to_string().into(),
        requirements.network.clone(),
        requirements.amount.inner().to_string().into(),
        requirements.pay_to.to_string().into(),
        requirements.asset.to_string().into(),
        requirements.max_timeout_seconds,
    )
    .with_optional_extra(requirements.extra.clone())
}

fn unix_now() -> Result<i64, ClientError> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| ClientError::Signing(err.to_string()))?
        .as_secs();
    i64::try_from(secs).map_err(|err| ClientError::Signing(err.to_string()))
}

fn sign_err(err: impl std::fmt::Display) -> ClientError {
    ClientError::Signing(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upto::payload::ASSET_TRANSFER_METHOD_CHANNEL;

    #[test]
    fn client_scheme_id_is_upto() {
        assert_eq!(SolanaUpto.scheme(), "upto");
        assert_eq!(ASSET_TRANSFER_METHOD_CHANNEL, "channel");
    }
}
