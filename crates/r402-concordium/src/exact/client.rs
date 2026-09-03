//! Client-side payment signing for the Concordium `"exact"` scheme.

use std::future::Future;
use std::pin::Pin;
use std::time::{SystemTime, UNIX_EPOCH};

use r402_client::{DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use r402_protocol::scheme::SchemeId;
use r402_protocol::{Base64Bytes, ChainId, ClientError, PaymentRequired, ResourceInfo};

use crate::chain::account::is_native_ccd;
use crate::chain::rpc::ConcordiumNode;
use crate::chain::signer::ConcordiumSigner;
use crate::chain::tx::{build_ccd_transfer, build_plt_transfer};
use crate::exact::payload;

/// Concordium exact scheme client.
#[derive(Clone)]
pub struct ConcordiumExactClient<S, N> {
    signer: S,
    node: N,
}

impl<S, N> std::fmt::Debug for ConcordiumExactClient<S, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConcordiumExactClient")
            .finish_non_exhaustive()
    }
}

impl<S, N> ConcordiumExactClient<S, N> {
    /// Creates a new Concordium exact client.
    pub const fn new(signer: S, node: N) -> Self {
        Self { signer, node }
    }
}

impl<S, N> SchemeId for ConcordiumExactClient<S, N> {
    fn namespace(&self) -> &'static str {
        "ccd"
    }

    fn scheme(&self) -> &'static str {
        "exact"
    }
}

impl<S, N> SchemeClient for ConcordiumExactClient<S, N>
where
    S: AsRef<ConcordiumSigner> + Send + Sync + Clone + 'static,
    N: ConcordiumNode + Clone + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: payload::v2::PaymentRequirements = v.as_concrete()?;
                let chain_id = requirements.network.clone();
                if chain_id.namespace() != "ccd" {
                    return None;
                }
                requirements.extra.as_ref()?.fee_payer?;
                Some(PaymentCandidate {
                    chain_id,
                    asset: requirements.asset.clone(),
                    amount: requirements.amount.clone(),
                    scheme: self.scheme().into(),
                    pay_to: requirements.pay_to.clone(),
                    requirements: v.clone(),
                    signer: Box::new(V2PayloadSigner {
                        signer: self.signer.clone(),
                        node: self.node.clone(),
                        requirements,
                        resource: payment_required.resource.clone(),
                    }),
                })
            })
            .collect()
    }

    fn find_default_asset(&self, asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
        crate::find_default_concordium_asset(asset, network)
    }
}

struct V2PayloadSigner<S, N> {
    signer: S,
    node: N,
    requirements: payload::v2::PaymentRequirements,
    resource: ResourceInfo,
}

impl<S, N> PaymentCandidateSigner for V2PayloadSigner<S, N>
where
    S: AsRef<ConcordiumSigner> + Send + Sync,
    N: ConcordiumNode,
{
    fn sign_payment(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + '_>> {
        Box::pin(async move {
            let signed =
                create_signed_transaction(self.signer.as_ref(), &self.node, &self.requirements)
                    .await?;
            let payload = payload::v2::PaymentPayload::new(self.requirements.clone(), signed)
                .with_resource(self.resource.clone());
            let json = serde_json::to_vec(&payload)?;
            Ok(Base64Bytes::encode(&json).to_string())
        })
    }
}

/// Builds a sender-signed V1 sponsored transaction for `requirements`.
///
/// # Errors
///
/// Missing `extra.feePayer`, invalid amounts, RPC failures, or signing errors.
pub async fn create_signed_transaction<N: ConcordiumNode>(
    signer: &ConcordiumSigner,
    node: &N,
    requirements: &payload::v2::PaymentRequirements,
) -> Result<payload::ExactConcordiumPayload, ClientError> {
    let fee_payer = requirements
        .extra
        .as_ref()
        .and_then(|e| e.fee_payer)
        .ok_or_else(|| {
            ClientError::Signing(
                "requirements.extra.feePayer is required. \
                 The resource server must include the facilitator's fee payer address in PaymentRequirements."
                    .to_owned(),
            )
        })?;
    if requirements.max_timeout_seconds <= 5 {
        return Err(ClientError::Signing(
            "requirements.maxTimeoutSeconds must be an integer greater than 5".to_owned(),
        ));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| ClientError::Signing(e.to_string()))?
        .as_secs();
    let expiry = now.saturating_add(requirements.max_timeout_seconds.saturating_sub(5));
    let nonce = node
        .next_nonce(&signer.address().to_string())
        .await
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let amount = requirements.amount.as_str();
    if amount.is_empty() || !amount.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ClientError::Signing(
            "amount must be a non-empty decimal string".to_owned(),
        ));
    }
    let pay_to = requirements.pay_to.as_str();
    if pay_to.is_empty() {
        return Err(ClientError::Signing("payTo address is required".to_owned()));
    }
    let tx = if is_native_ccd(requirements.asset.as_str()) {
        build_ccd_transfer(
            signer,
            pay_to,
            amount,
            &fee_payer.to_string(),
            nonce,
            expiry,
        )?
    } else {
        let decimals = node
            .get_token_decimals(requirements.asset.as_str())
            .await
            .map_err(|e| ClientError::Signing(e.to_string()))?;
        build_plt_transfer(
            signer,
            pay_to,
            amount,
            requirements.asset.as_str(),
            decimals,
            &fee_payer.to_string(),
            nonce,
            expiry,
        )?
    };
    Ok(payload::ExactConcordiumPayload {
        signed_transaction: tx,
        sender: Some(signer.address().to_string()),
    })
}
