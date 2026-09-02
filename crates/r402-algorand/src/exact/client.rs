//! Client-side payment signing for the Algorand `"exact"` scheme.

use base64::Engine;
use r402_core::chain::ChainId;
use r402_core::error::ClientError;
use r402_core::scheme::SchemeId;
use r402_core::scheme::{DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use r402_core::wire::Base64Bytes;
use r402_core::wire::PaymentRequired;
use r402_core::wire::ResourceInfo;

use crate::DEFAULT_VALIDITY_ROUNDS;
use crate::chain::codec::{
    SignedTransaction, Transaction, TxnType, assign_group, encode_signed, encode_txn, txn_fee,
};
use crate::chain::rpc::AlgodRpc;
use crate::chain::signer::AlgorandSigner;
use crate::chain::types::{AlgorandAddress, AlgorandChainReference, normalize_algorand_network};
use crate::exact::types;
use crate::exact::{AlgorandExact, ExactAvmPayload};

/// Builds a base64 atomic group: optional unsigned fee-payer pay + signed ASA transfer.
///
/// # Errors
///
/// Returns [`ClientError`] if RPC reads fail, addresses are invalid, or the
/// suggested-params genesis hash does not match the network.
pub async fn create_payment_group<R: AlgodRpc>(
    signer: &AlgorandSigner,
    rpc: &R,
    pay_to: AlgorandAddress,
    asset_id: u64,
    amount: u64,
    fee_payer: Option<AlgorandAddress>,
    chain: AlgorandChainReference,
) -> Result<ExactAvmPayload, ClientError> {
    let params = rpc
        .suggested_params()
        .await
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    if params.genesis_hash != chain.genesis_hash() {
        return Err(ClientError::Signing(
            "algod genesis hash does not match payment network".to_owned(),
        ));
    }

    let first_valid = params.last_round;
    let last_valid = params.last_round.saturating_add(DEFAULT_VALIDITY_ROUNDS);

    let mut txns = Vec::new();
    let payment_index = if let Some(fee_payer) = fee_payer {
        let mut pay = Transaction::new(TxnType::Pay, fee_payer);
        pay.receiver = Some(fee_payer);
        pay.fee = 0;
        pay.first_valid = first_valid;
        pay.last_valid = last_valid;
        pay.genesis_id.clone_from(&params.genesis_id);
        pay.genesis_hash = chain.genesis_hash();
        txns.push(pay);
        1_u32
    } else {
        0_u32
    };

    let mut axfer = Transaction::new(TxnType::Axfer, signer.address());
    axfer.asset_id = asset_id;
    axfer.asset_amount = amount;
    axfer.asset_receiver = Some(pay_to);
    axfer.first_valid = first_valid;
    axfer.last_valid = last_valid;
    axfer.genesis_id = params.genesis_id;
    axfer.genesis_hash = chain.genesis_hash();
    if fee_payer.is_some() {
        axfer.fee = 0;
    }
    txns.push(axfer);
    txns = assign_group(txns);

    if fee_payer.is_some() {
        let mut total_fee = 0_u64;
        for txn in &txns {
            let size = encode_txn(txn).len();
            total_fee =
                total_fee.saturating_add(txn_fee(params.fee_per_byte, params.min_fee, size));
        }
        if let Some(pay) = txns.first_mut() {
            pay.fee = total_fee;
        }
        // Fee is part of the digest that produces `grp`, so regroup after the update.
        txns = assign_group(txns);
    } else if let Some(txn) = txns.first_mut() {
        let size = encode_txn(txn).len();
        txn.fee = txn_fee(params.fee_per_byte, params.min_fee, size);
    }

    let mut payment_group = Vec::with_capacity(txns.len());
    for txn in &txns {
        let encoded = if txn.sender == signer.address() {
            encode_signed(&signer.sign(txn))
        } else {
            encode_signed(&SignedTransaction {
                sig: None,
                txn: txn.clone(),
            })
        };
        payment_group.push(base64::engine::general_purpose::STANDARD.encode(encoded));
    }

    Ok(ExactAvmPayload {
        payment_group,
        payment_index,
    })
}

/// Algorand exact scheme client for building and signing payment payloads.
#[derive(Clone)]
pub struct AlgorandExactClient<S, R> {
    signer: S,
    rpc: R,
}

impl<S, R> std::fmt::Debug for AlgorandExactClient<S, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlgorandExactClient")
            .finish_non_exhaustive()
    }
}

impl<S, R> AlgorandExactClient<S, R> {
    /// Creates a new Algorand exact client.
    pub const fn new(signer: S, rpc: R) -> Self {
        Self { signer, rpc }
    }
}

impl<S, R> SchemeId for AlgorandExactClient<S, R> {
    fn namespace(&self) -> &str {
        AlgorandExact.namespace()
    }

    fn scheme(&self) -> &str {
        AlgorandExact.scheme()
    }
}

impl<S, R> r402_core::scheme::Sealed for AlgorandExactClient<S, R> {}

impl<S, R> SchemeClient for AlgorandExactClient<S, R>
where
    S: AsRef<AlgorandSigner> + Send + Sync + Clone + 'static,
    R: AlgodRpc + Clone + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: types::v2::PaymentRequirements = v.as_concrete()?;
                let chain_id = requirements.network.clone();
                if chain_id.namespace() != "algorand" {
                    return None;
                }
                Some(PaymentCandidate {
                    chain_id,
                    asset: requirements.asset.clone(),
                    amount: requirements.amount.to_string().into(),
                    scheme: self.scheme().into(),
                    pay_to: requirements.pay_to.clone(),
                    requirements: v.clone(),
                    signer: Box::new(V2PayloadSigner {
                        signer: self.signer.clone(),
                        rpc: self.rpc.clone(),
                        requirements,
                        resource: payment_required.resource.clone(),
                    }),
                })
            })
            .collect()
    }

    fn find_default_asset(&self, asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
        crate::find_default_algorand_asset(asset, network)
    }
}

impl AsRef<Self> for AlgorandSigner {
    fn as_ref(&self) -> &Self {
        self
    }
}

struct V2PayloadSigner<S, R> {
    signer: S,
    rpc: R,
    requirements: types::v2::PaymentRequirements,
    resource: ResourceInfo,
}

impl<S, R> PaymentCandidateSigner for V2PayloadSigner<S, R>
where
    S: AsRef<AlgorandSigner> + Send + Sync,
    R: AlgodRpc,
{
    fn sign_payment(&self) -> r402_core::facilitator::BoxFuture<'_, Result<String, ClientError>> {
        Box::pin(async move {
            let chain = normalize_algorand_network(&self.requirements.network.to_string())
                .ok_or_else(|| ClientError::Signing("unsupported algorand network".to_owned()))?;
            let pay_to: AlgorandAddress = self
                .requirements
                .pay_to
                .parse()
                .map_err(|e| ClientError::Signing(format!("{e}")))?;
            let asset_id: u64 = self
                .requirements
                .asset
                .parse()
                .map_err(|e| ClientError::Signing(format!("invalid ASA id: {e}")))?;
            let fee_payer = self.requirements.extra.as_ref().and_then(|e| e.fee_payer);
            let payload = create_payment_group(
                self.signer.as_ref(),
                &self.rpc,
                pay_to,
                asset_id,
                self.requirements.amount.inner(),
                fee_payer,
                chain,
            )
            .await?;
            let payload = types::v2::PaymentPayload::new(self.requirements.clone(), payload)
                .with_resource(self.resource.clone());
            let json = serde_json::to_vec(&payload)?;
            let encoded = Base64Bytes::encode(&json);
            Ok(encoded.to_string())
        })
    }
}
