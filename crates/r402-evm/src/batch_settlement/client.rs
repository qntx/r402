//! Client signing for EVM `batch-settlement` (voucher path).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use alloy_primitives::{Address, B256, U256};
use r402_client::{DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use r402_protocol::error::ClientError;
use r402_protocol::network::ChainId;
use r402_protocol::payment::{Base64Bytes, PaymentRequired, ResourceInfo};
use r402_protocol::scheme::SchemeId;

use super::channel::{build_channel_config, compute_channel_id};
use super::payload::{BatchSettlementExtra, BatchSettlementPayload, ChannelConfig};
use super::voucher::sign_voucher;
use super::{Eip155BatchSettlement, payload};
use crate::chain::Eip155ChainReference;
use crate::signer::SignerLike;

/// Client that signs cumulative vouchers for batch-settlement channels.
///
/// Tracks last charged cumulative per channel so the next voucher ceiling is
/// `charged + amount`. Callers can seed state via [`Self::set_charged`].
pub struct Eip155BatchSettlementClient<S> {
    signer: S,
    /// Optional fixed payer authorizer (defaults to signer address).
    payer_authorizer: Option<Address>,
    /// In-process charged totals (`channel_id` → amount).
    charged: Arc<Mutex<HashMap<B256, U256>>>,
}

impl<S: std::fmt::Debug> std::fmt::Debug for Eip155BatchSettlementClient<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Eip155BatchSettlementClient")
            .finish_non_exhaustive()
    }
}

impl<S> Eip155BatchSettlementClient<S> {
    /// New client.
    #[must_use]
    pub fn new(signer: S) -> Self {
        Self {
            signer,
            payer_authorizer: None,
            charged: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Seeds charged cumulative for a channel (recovery / resync).
    pub fn set_charged(&self, channel_id: B256, amount: U256) {
        let _ = self
            .charged
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(channel_id, amount);
    }

    /// Reads charged cumulative (zero if unknown).
    #[must_use]
    pub fn charged(&self, channel_id: &B256) -> U256 {
        self.charged
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(channel_id)
            .copied()
            .unwrap_or(U256::ZERO)
    }
}

impl<S> SchemeId for Eip155BatchSettlementClient<S> {
    fn namespace(&self) -> &'static str {
        Eip155BatchSettlement.namespace()
    }

    fn scheme(&self) -> &str {
        Eip155BatchSettlement.scheme()
    }
}

impl<S> SchemeClient for Eip155BatchSettlementClient<S>
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
                    signer: Box::new(BatchVoucherSigner {
                        signer: self.signer.clone(),
                        charged: Arc::clone(&self.charged),
                        resource_info: Some(payment_required.resource.clone()),
                        payer_authorizer: self.payer_authorizer,
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

struct BatchVoucherSigner<S> {
    signer: S,
    charged: Arc<Mutex<HashMap<B256, U256>>>,
    resource_info: Option<ResourceInfo>,
    payer_authorizer: Option<Address>,
    chain_id: u64,
    requirements: payload::v2::PaymentRequirements,
    extra: BatchSettlementExtra,
}

impl<S> PaymentCandidateSigner for BatchVoucherSigner<S>
where
    S: SignerLike + Sync,
{
    fn sign_payment<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + 'a>> {
        Box::pin(async move {
            let payer = self.signer.address();
            let authorizer = self.payer_authorizer.unwrap_or(payer);
            // Zero salt → stable channel id for a given (payer, receiver, token) set
            // without client-side channel persistence.
            let salt = B256::ZERO;

            let config = build_channel_config(
                payer,
                authorizer,
                self.requirements.pay_to.0,
                self.extra.receiver_authorizer.0,
                self.requirements.asset.0,
                self.extra.withdraw_delay,
                salt,
            );
            let channel_id = compute_channel_id(&config, self.chain_id)
                .map_err(|e| ClientError::Signing(e.to_string()))?;
            let charged = self
                .charged
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&channel_id)
                .copied()
                .unwrap_or(U256::ZERO);
            let max_claimable = charged.saturating_add(self.requirements.amount.0);
            let voucher =
                sign_voucher(&self.signer, channel_id, max_claimable, self.chain_id).await?;
            let scheme_payload = BatchSettlementPayload::Voucher {
                channel_config: config,
                voucher,
            };
            let payload =
                payload::v2::PaymentPayload::new(self.requirements.clone(), scheme_payload)
                    .with_optional_resource(self.resource_info.clone());
            let json = serde_json::to_vec(&payload)?;
            Ok(Base64Bytes::encode(&json).to_string())
        })
    }
}

/// Signs a voucher payload for an explicit channel config.
///
/// # Errors
///
/// Signing failure.
pub async fn sign_voucher_payload<S: SignerLike + Sync>(
    signer: &S,
    config: &ChannelConfig,
    max_claimable: U256,
    chain_id: u64,
) -> Result<BatchSettlementPayload, ClientError> {
    let channel_id =
        compute_channel_id(config, chain_id).map_err(|e| ClientError::Signing(e.to_string()))?;
    let voucher = sign_voucher(signer, channel_id, max_claimable, chain_id).await?;
    Ok(BatchSettlementPayload::Voucher {
        channel_config: *config,
        voucher,
    })
}
