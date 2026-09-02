//! Client signing for EVM `batch-settlement` (voucher path).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use alloy_primitives::{Address, B256, U256};
use r402_core::chain::ChainId;
use r402_core::error::ClientError;
use r402_core::scheme::{
    DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner, SchemeClient, SchemeId, Sealed,
};
use r402_core::wire::{Base64Bytes, PaymentRequired};

use super::channel::{build_channel_config, compute_channel_id};
use super::types::{BatchSettlementExtra, BatchSettlementPayload, ChannelConfig};
use super::voucher::sign_voucher;
use crate::signer::SignerLike;

/// Client that signs cumulative vouchers for batch-settlement channels.
///
/// Tracks last charged cumulative per channel so the next voucher ceiling is
/// `charged + amount`. Callers can seed state via [`Self::set_charged`].
pub struct Eip155BatchSettlementClient<S> {
    signer: Arc<S>,
    /// Optional fixed payer authorizer (defaults to signer address).
    payer_authorizer: Option<Address>,
    /// In-process charged totals (`channel_id` → amount).
    charged: Arc<std::sync::Mutex<std::collections::HashMap<B256, U256>>>,
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
            signer: Arc::new(signer),
            payer_authorizer: None,
            charged: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
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

impl<S: SignerLike + Sync + 'static> SchemeId for Eip155BatchSettlementClient<S> {
    fn namespace(&self) -> &'static str {
        "eip155"
    }

    fn scheme(&self) -> &'static str {
        "batch-settlement"
    }
}

impl<S: SignerLike + Sync + 'static> Sealed for Eip155BatchSettlementClient<S> {}

impl<S: SignerLike + Sync + 'static> SchemeClient for Eip155BatchSettlementClient<S> {
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        let mut out = Vec::new();
        for req in &payment_required.accepts {
            if req.scheme.as_str() != "batch-settlement" {
                continue;
            }
            if !req.network.namespace().eq_ignore_ascii_case("eip155") {
                continue;
            }
            let Some(extra_val) = req.extra.clone() else {
                continue;
            };
            let Ok(extra) = serde_json::from_value::<BatchSettlementExtra>(extra_val) else {
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
                scheme: "batch-settlement".into(),
                pay_to: req.pay_to.clone(),
                requirements: req.clone(),
                signer: Box::new(BatchVoucherSigner {
                    signer: Arc::clone(&self.signer),
                    charged: Arc::clone(&self.charged),
                    payer_authorizer: self.payer_authorizer,
                    chain_id,
                    asset,
                    pay_to,
                    amount,
                    extra,
                }),
            });
        }
        out
    }

    fn find_default_asset(&self, asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
        crate::find_default_evm_asset_info(asset, network)
    }
}

struct BatchVoucherSigner<S> {
    signer: Arc<S>,
    charged: Arc<std::sync::Mutex<std::collections::HashMap<B256, U256>>>,
    payer_authorizer: Option<Address>,
    chain_id: u64,
    asset: Address,
    pay_to: Address,
    amount: U256,
    extra: BatchSettlementExtra,
}

impl<S: SignerLike + Sync + 'static> PaymentCandidateSigner for BatchVoucherSigner<S> {
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
                self.pay_to,
                self.extra.receiver_authorizer.0,
                self.asset,
                self.extra.withdraw_delay,
                salt,
            );
            let channel_id = compute_channel_id(&config, self.chain_id);
            let charged = self
                .charged
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&channel_id)
                .copied()
                .unwrap_or(U256::ZERO);
            let max_claimable = charged.saturating_add(self.amount);
            let voucher = sign_voucher(
                self.signer.as_ref(),
                channel_id,
                max_claimable,
                self.chain_id,
            )
            .await?;
            let payload = BatchSettlementPayload::Voucher {
                channel_config: config,
                voucher,
            };
            let json = serde_json::to_vec(&payload).map_err(ClientError::Json)?;
            Ok(Base64Bytes::encode(json).to_string())
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
    let channel_id = compute_channel_id(config, chain_id);
    let voucher = sign_voucher(signer, channel_id, max_claimable, chain_id).await?;
    Ok(BatchSettlementPayload::Voucher {
        channel_config: *config,
        voucher,
    })
}
