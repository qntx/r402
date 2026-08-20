//! Client-side payment signing for the Stellar `"exact"` scheme.

use r402_core::error::ClientError;
use r402_core::facilitator::BoxFuture;
use r402_core::scheme::SchemeId;
use r402_core::scheme::{PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use r402_core::wire::Base64Bytes;
use r402_core::wire::PaymentRequired;
use r402_core::wire::ResourceInfo;
use stellar_xdr::TransactionExt;

use crate::chain::rpc::StellarRpc;
use crate::chain::signer::StellarSigner;
use crate::chain::xdr::{
    build_transfer_envelope, decode_transaction_envelope, encode_transaction_envelope,
    gather_auth_entry_signature_status, inner_transaction, inner_transaction_mut,
    invoke_host_function_op_mut,
};
use crate::chain::{StellarChainReference, parse_contract_address};
use crate::exact::types;
use crate::exact::{ExactStellarPayload, StellarExact, StellarExtra};
use crate::{BASE_FEE_STROOPS, DEFAULT_TIMEOUT_SECONDS, NULL_ACCOUNT, timeout_ledgers};

/// Builds a base64 transaction XDR with signed auth entries only.
///
/// # Errors
///
/// Returns [`ClientError`] if RPC, simulation, or signing fails.
#[allow(
    clippy::too_many_arguments,
    reason = "mirrors the TS createPaymentPayload field list"
)]
pub async fn create_signed_payment_transaction<R: StellarRpc>(
    signer: &StellarSigner,
    rpc: &R,
    chain: StellarChainReference,
    pay_to: &str,
    asset: &str,
    amount: i128,
    max_timeout_seconds: u64,
    extra: &StellarExtra,
) -> Result<String, ClientError> {
    if !extra.are_fees_sponsored {
        return Err(ClientError::Signing(
            "Exact scheme requires areFeesSponsored to be true".to_owned(),
        ));
    }
    if amount <= 0 {
        return Err(ClientError::Signing(format!(
            "Invalid amount: {amount}. Amount must be a positive integer."
        )));
    }
    parse_contract_address(asset).map_err(|e| ClientError::Signing(e.to_string()))?;
    let _pay_to: crate::chain::StellarAddress =
        pay_to
            .parse()
            .map_err(|e: crate::chain::StellarAddressFormatError| {
                ClientError::Signing(e.to_string())
            })?;

    let current_ledger = rpc
        .latest_ledger()
        .await
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let estimated = rpc.estimated_ledger_seconds().await;
    let expiration = current_ledger.saturating_add(timeout_ledgers(max_timeout_seconds, estimated));
    let now = unix_now();

    let mut envelope = build_transfer_envelope(
        NULL_ACCOUNT,
        asset,
        signer.address(),
        pay_to,
        amount,
        0,
        BASE_FEE_STROOPS,
        Vec::new(),
        TransactionExt::V0,
        max_timeout_seconds,
        now,
    )
    .map_err(|e| ClientError::Signing(e.to_string()))?;

    let simulation = rpc
        .simulate_transaction(&envelope)
        .await
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    if simulation.error.is_some() || simulation.restore_preamble.is_some() {
        return Err(ClientError::Signing(format!(
            "Stellar simulation failed{}",
            simulation
                .error
                .as_deref()
                .map(|e| format!(" with error message: {e}"))
                .unwrap_or_default()
        )));
    }
    let results = simulation
        .results()
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let mut auth = results.first().map(|r| r.auth.clone()).unwrap_or_default();
    let pending = gather_auth_entry_signature_status(&auth);
    if !pending
        .pending_signature
        .contains(&signer.address().to_owned())
        || pending.pending_signature.len() != 1
    {
        return Err(ClientError::Signing(format!(
            "Expected to sign with [{}], but got [{}]",
            signer.address(),
            pending.pending_signature.join(", ")
        )));
    }
    signer
        .sign_auth_entries(&mut auth, chain.passphrase(), expiration)
        .map_err(|e| ClientError::Signing(e.to_string()))?;

    {
        let tx = inner_transaction_mut(&mut envelope)
            .map_err(|e| ClientError::Signing(e.to_string()))?;
        let op =
            invoke_host_function_op_mut(tx).map_err(|e| ClientError::Signing(e.to_string()))?;
        op.auth = auth
            .try_into()
            .map_err(|e: stellar_xdr::Error| ClientError::Signing(e.to_string()))?;
        if let Ok(data) = simulation.transaction_data() {
            let resource_fee = u64::try_from(data.resource_fee.max(0)).unwrap_or(0);
            tx.fee = u32::try_from(u64::from(BASE_FEE_STROOPS).saturating_add(resource_fee))
                .unwrap_or(u32::MAX);
            tx.ext = TransactionExt::V1(data);
        }
    }

    let confirm = rpc
        .simulate_transaction(&envelope)
        .await
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    if confirm.error.is_some() || confirm.restore_preamble.is_some() {
        return Err(ClientError::Signing(
            "Stellar simulation failed after signing auth entries".to_owned(),
        ));
    }
    let tx = inner_transaction(&envelope).map_err(|e| ClientError::Signing(e.to_string()))?;
    let op = crate::chain::xdr::invoke_host_function_op(tx)
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let leftover = gather_auth_entry_signature_status(&op.auth);
    if !leftover.pending_signature.is_empty() {
        return Err(ClientError::Signing(format!(
            "unexpected signer(s) required: [{}]",
            leftover.pending_signature.join(", ")
        )));
    }

    encode_transaction_envelope(&envelope).map_err(|e| ClientError::Signing(e.to_string()))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Stellar exact scheme client for building and signing payment payloads.
#[derive(Clone)]
pub struct StellarExactClient<S, R> {
    signer: S,
    rpc: R,
    chain: StellarChainReference,
}

impl<S, R> std::fmt::Debug for StellarExactClient<S, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StellarExactClient")
            .field("chain", &self.chain)
            .finish_non_exhaustive()
    }
}

impl<S, R> StellarExactClient<S, R> {
    /// Creates a new Stellar exact client bound to `chain`.
    pub const fn new(signer: S, rpc: R, chain: StellarChainReference) -> Self {
        Self { signer, rpc, chain }
    }
}

impl<S, R> SchemeId for StellarExactClient<S, R> {
    fn namespace(&self) -> &str {
        StellarExact.namespace()
    }

    fn scheme(&self) -> &str {
        StellarExact.scheme()
    }
}

impl<S, R> r402_core::scheme::Sealed for StellarExactClient<S, R> {}

impl<S, R> SchemeClient for StellarExactClient<S, R>
where
    S: AsRef<StellarSigner> + Send + Sync + Clone + 'static,
    R: StellarRpc + Clone + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: types::v2::PaymentRequirements = v.as_concrete()?;
                let chain_id = requirements.network.clone();
                if chain_id.namespace() != "stellar" {
                    return None;
                }
                Some(PaymentCandidate {
                    chain_id,
                    asset: requirements.asset.to_string().into(),
                    amount: requirements.amount.as_str().into(),
                    scheme: self.scheme().into(),
                    pay_to: requirements.pay_to.to_string().into(),
                    signer: Box::new(V2PayloadSigner {
                        signer: self.signer.clone(),
                        rpc: self.rpc.clone(),
                        chain: self.chain,
                        requirements,
                        resource: payment_required.resource.clone(),
                    }),
                })
            })
            .collect()
    }
}

struct V2PayloadSigner<S, R> {
    signer: S,
    rpc: R,
    chain: StellarChainReference,
    requirements: types::v2::PaymentRequirements,
    resource: ResourceInfo,
}

impl<S, R> PaymentCandidateSigner for V2PayloadSigner<S, R>
where
    S: AsRef<StellarSigner> + Send + Sync,
    R: StellarRpc,
{
    fn sign_payment(&self) -> BoxFuture<'_, Result<String, ClientError>> {
        Box::pin(async move {
            let extra = self.requirements.extra.ok_or_else(|| {
                ClientError::Signing("Exact scheme requires areFeesSponsored to be true".to_owned())
            })?;
            let amount = self
                .requirements
                .amount
                .as_i128()
                .map_err(|e| ClientError::Signing(e.to_string()))?;
            let timeout = if self.requirements.max_timeout_seconds == 0 {
                DEFAULT_TIMEOUT_SECONDS
            } else {
                self.requirements.max_timeout_seconds
            };
            let b64 = create_signed_payment_transaction(
                self.signer.as_ref(),
                &self.rpc,
                self.chain,
                self.requirements.pay_to.as_str(),
                self.requirements.asset.as_str(),
                amount,
                timeout,
                &extra,
            )
            .await?;
            let payload = types::v2::PaymentPayload::new(
                self.requirements.clone(),
                ExactStellarPayload { transaction: b64 },
            )
            .with_resource(self.resource.clone());
            let json = serde_json::to_vec(&payload)?;
            let encoded = Base64Bytes::encode(&json);
            Ok(encoded.to_string())
        })
    }
}

/// Decodes a payment-payload transaction XDR. Exposed for tests.
///
/// # Errors
///
/// Returns [`ClientError`] when the XDR is invalid.
pub fn decode_payment_transaction(
    xdr_b64: &str,
) -> Result<stellar_xdr::TransactionEnvelope, ClientError> {
    decode_transaction_envelope(xdr_b64).map_err(|e| ClientError::Signing(e.to_string()))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::unreachable,
    clippy::manual_async_fn,
    clippy::panic,
    reason = "test assertions"
)]
mod tests {
    use std::future::Future;

    use super::*;
    use crate::exact::StellarExtra;

    struct UnreachableRpc;

    impl StellarRpc for UnreachableRpc {
        fn latest_ledger(
            &self,
        ) -> impl Future<Output = Result<u32, crate::chain::StellarRpcError>> + Send {
            async { unreachable!("extra is checked before RPC") }
        }

        fn get_account(
            &self,
            _address: &str,
        ) -> impl Future<Output = Result<stellar_xdr::AccountEntry, crate::chain::StellarRpcError>> + Send
        {
            async { unreachable!("extra is checked before RPC") }
        }

        fn simulate_transaction(
            &self,
            _tx: &stellar_xdr::TransactionEnvelope,
        ) -> impl Future<
            Output = Result<
                stellar_rpc_client::SimulateTransactionResponse,
                crate::chain::StellarRpcError,
            >,
        > + Send {
            async { unreachable!("extra is checked before RPC") }
        }

        fn send_transaction(
            &self,
            _tx: &stellar_xdr::TransactionEnvelope,
        ) -> impl Future<Output = Result<stellar_xdr::Hash, crate::chain::StellarRpcError>> + Send
        {
            async { unreachable!("extra is checked before RPC") }
        }

        fn get_transaction(
            &self,
            _hash: &stellar_xdr::Hash,
        ) -> impl Future<
            Output = Result<
                stellar_rpc_client::GetTransactionResponse,
                crate::chain::StellarRpcError,
            >,
        > + Send {
            async { unreachable!("extra is checked before RPC") }
        }

        fn poll_transaction(
            &self,
            _hash: &stellar_xdr::Hash,
            _timeout: std::time::Duration,
        ) -> impl Future<
            Output = Result<
                stellar_rpc_client::GetTransactionResponse,
                crate::chain::StellarRpcError,
            >,
        > + Send {
            async { unreachable!("extra is checked before RPC") }
        }

        fn estimated_ledger_seconds(&self) -> impl Future<Output = u64> + Send {
            async { unreachable!("extra is checked before RPC") }
        }
    }

    #[tokio::test]
    async fn rejects_unsponsored_extra_without_rpc() {
        let signer =
            StellarSigner::from_secret("SCKB3ECHCPVM4HJPNCQWTQWJJ5XRL6UNKLTTCIH4B7TB22NKJ5GUFMIV")
                .unwrap();
        let extra = StellarExtra {
            are_fees_sponsored: false,
        };
        let err = create_signed_payment_transaction(
            &signer,
            &UnreachableRpc,
            StellarChainReference::TESTNET,
            "GCHEI4PQEFJOA27MNZRPQNLGURS6KASW76X5UZCUZIXCOJLKXYCXOR2W",
            crate::USDC_TESTNET_ADDRESS,
            1,
            60,
            &extra,
        )
        .await
        .unwrap_err();
        match err {
            ClientError::Signing(msg) => {
                assert!(
                    msg.contains("areFeesSponsored"),
                    "unexpected signing error: {msg}"
                );
            }
            other => panic!("expected Signing, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn live_sign_skipped_without_env() {
        if std::env::var("STELLAR_SECRET_KEY").is_err() {
            return;
        }
        let secret = std::env::var("STELLAR_SECRET_KEY").unwrap();
        let signer = StellarSigner::from_secret(&secret).unwrap();
        let rpc =
            crate::chain::StellarJsonRpc::connect(StellarChainReference::TESTNET, None).unwrap();
        let extra = StellarExtra::sponsored();
        let result = create_signed_payment_transaction(
            &signer,
            &rpc,
            StellarChainReference::TESTNET,
            "GCHEI4PQEFJOA27MNZRPQNLGURS6KASW76X5UZCUZIXCOJLKXYCXOR2W",
            crate::USDC_TESTNET_ADDRESS,
            1,
            60,
            &extra,
        )
        .await;
        assert!(result.is_ok(), "live sign failed: {result:?}");
    }
}
