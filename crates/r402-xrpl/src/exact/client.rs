//! Client-side payment signing for the XRPL `"exact"` scheme.

use r402_core::chain::ChainId;
use r402_core::error::ClientError;
use r402_core::scheme::SchemeId;
use r402_core::scheme::{DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use r402_core::wire::Base64Bytes;
use r402_core::wire::PaymentRequired;
use r402_core::wire::ResourceInfo;
use serde_json::{Value, json};
use xrpl::core::keypairs::{derive_classic_address, derive_keypair};

use crate::chain::codec::{
    invoice_id_to_field, max_last_ledger_sequence, sign_transaction, signed_tx_hash,
};
use crate::chain::rpc::{XrplRpc, extract_created_ticket_sequences, submit_and_wait};
use crate::chain::types::{
    XrplChainReference, is_decimal_string, is_integer_string, is_xrpl_network,
    parse_xrpl_network_id,
};
use crate::exact::types;
use crate::exact::{ExactXrplPayload, XrplAssetTransferMethod, XrplExact, XrplExtra};
use crate::{DEFAULT_MAX_FEE_DROPS, MAX_ACCOUNT_TICKETS};

/// Local XRPL signer wrapping a hex key pair derived from a family seed.
#[derive(Clone)]
pub struct XrplSigner {
    classic_address: String,
    public_key: String,
    private_key: String,
}

impl std::fmt::Debug for XrplSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XrplSigner")
            .field("classic_address", &self.classic_address)
            .field("public_key", &self.public_key)
            .finish_non_exhaustive()
    }
}

impl XrplSigner {
    /// Constructs a signer from an XRPL family seed (`s…` / `sEd…`).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Signing`] if the seed is invalid.
    pub fn from_seed(seed: impl AsRef<str>) -> Result<Self, ClientError> {
        let (public_key, private_key) = derive_keypair(seed.as_ref(), false)
            .map_err(|e| ClientError::Signing(e.to_string()))?;
        let classic_address =
            derive_classic_address(&public_key).map_err(|e| ClientError::Signing(e.to_string()))?;
        Ok(Self {
            classic_address,
            public_key,
            private_key,
        })
    }

    /// Classic address this signer pays from.
    #[must_use]
    pub fn classic_address(&self) -> &str {
        &self.classic_address
    }

    /// Hex-encoded signing public key (`02`/`03`/`ED…`).
    #[must_use]
    pub fn public_key(&self) -> &str {
        &self.public_key
    }
}

/// Optional overrides for locally built payments.
#[derive(Debug, Clone, Default)]
pub struct XrplClientOptions {
    /// Fee in drops; when omitted the open-ledger fee is fetched.
    pub fee_drops: Option<String>,
    /// Tickets to create when none are available. `0` disables auto-create.
    pub ticket_create_count: Option<u32>,
}

/// Builds and signs an XRPL `Payment` for the given requirements.
///
/// # Errors
///
/// Returns [`ClientError`] if requirements are invalid, RPC fails, or signing fails.
pub async fn create_signed_payment<R: XrplRpc>(
    signer: &XrplSigner,
    rpc: &R,
    requirements: &types::v2::PaymentRequirements,
    options: &XrplClientOptions,
) -> Result<String, ClientError> {
    validate_requirements(requirements)?;
    let extra = requirements.extra.as_ref();
    let method = extra
        .and_then(|e| e.asset_transfer_method)
        .unwrap_or(XrplAssetTransferMethod::Sequence);
    let mut tx = build_payment_json(signer, requirements, extra)?;
    fill_ledger_fields(&mut tx, signer, rpc, requirements, method, options).await?;
    sign_transaction(&mut tx, &signer.private_key, &signer.public_key)
        .map_err(|e| ClientError::Signing(e.to_string()))
}

fn validate_requirements(requirements: &types::v2::PaymentRequirements) -> Result<(), ClientError> {
    if !is_xrpl_network(&requirements.network.to_string()) {
        return Err(ClientError::Signing(format!(
            "unsupported xrpl network: {}",
            requirements.network
        )));
    }
    let extra = requirements.extra.as_ref().ok_or_else(|| {
        ClientError::Signing(
            "XRPL exact payments require extra.areFeesSponsored to be false".to_owned(),
        )
    })?;
    if extra.are_fees_sponsored {
        return Err(ClientError::Signing(
            "XRPL exact payments require extra.areFeesSponsored to be false; the payer pays the XRPL transaction fee".to_owned(),
        ));
    }
    if let Some(invoice) = extra.invoice_id.as_ref()
        && invoice.is_empty()
    {
        return Err(ClientError::Signing(
            "XRPL exact payments require a non-empty extra.invoiceId when provided".to_owned(),
        ));
    }
    if requirements.asset.is_xrp() {
        if !is_integer_string(requirements.amount.as_str()) {
            return Err(ClientError::Signing(
                "XRPL native payments require amount as an integer drops string".to_owned(),
            ));
        }
        return Ok(());
    }
    if extra.issuer.is_none() {
        return Err(ClientError::Signing(
            "XRPL IOU payments require extra.issuer".to_owned(),
        ));
    }
    if !is_decimal_string(requirements.amount.as_str()) {
        return Err(ClientError::Signing(
            "XRPL IOU payments require amount as an issued-currency decimal value string"
                .to_owned(),
        ));
    }
    Ok(())
}

fn build_payment_json(
    signer: &XrplSigner,
    requirements: &types::v2::PaymentRequirements,
    extra: Option<&XrplExtra>,
) -> Result<Value, ClientError> {
    let network_id = parse_xrpl_network_id(&requirements.network.to_string())
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let mut tx = json!({
        "TransactionType": "Payment",
        "Account": signer.classic_address,
        "Destination": requirements.pay_to.as_str(),
    });
    let obj = tx
        .as_object_mut()
        .ok_or_else(|| ClientError::Signing("payment json".to_owned()))?;
    if requirements.asset.is_xrp() {
        obj.insert(
            "Amount".to_owned(),
            Value::String(requirements.amount.as_str().to_owned()),
        );
    } else {
        let issuer = extra.and_then(|e| e.issuer.as_ref()).ok_or_else(|| {
            ClientError::Signing("XRPL IOU payments require extra.issuer".to_owned())
        })?;
        let amount = json!({
            "currency": requirements.asset.as_str(),
            "issuer": issuer.as_str(),
            "value": requirements.amount.as_str(),
        });
        obj.insert("Amount".to_owned(), amount.clone());
        obj.insert("SendMax".to_owned(), amount);
    }
    if let Some(invoice) = extra.and_then(|e| e.invoice_id.as_deref()) {
        obj.insert(
            "InvoiceID".to_owned(),
            Value::String(invoice_id_to_field(invoice)),
        );
    }
    if let Some(tag) = extra.and_then(|e| e.destination_tag) {
        obj.insert("DestinationTag".to_owned(), json!(tag));
    }
    if network_id > 1024 {
        obj.insert("NetworkID".to_owned(), json!(network_id));
    }
    Ok(tx)
}

async fn fill_ledger_fields<R: XrplRpc>(
    tx: &mut Value,
    signer: &XrplSigner,
    rpc: &R,
    requirements: &types::v2::PaymentRequirements,
    method: XrplAssetTransferMethod,
    options: &XrplClientOptions,
) -> Result<(), ClientError> {
    let current = rpc
        .current_ledger_index()
        .await
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let last_ledger = max_last_ledger_sequence(current, requirements.max_timeout_seconds);
    let fee = match options.fee_drops.clone() {
        Some(fee) => fee,
        None => rpc
            .fee_drops()
            .await
            .map_err(|e| ClientError::Signing(e.to_string()))?
            .min(DEFAULT_MAX_FEE_DROPS)
            .to_string(),
    };
    let obj = tx
        .as_object_mut()
        .ok_or_else(|| ClientError::Signing("payment json".to_owned()))?;
    obj.insert("Fee".to_owned(), Value::String(fee));
    obj.insert("LastLedgerSequence".to_owned(), json!(last_ledger));
    match method {
        XrplAssetTransferMethod::Sequence => {
            let auth = rpc
                .account_authorization(signer.classic_address())
                .await
                .map_err(|e| ClientError::Signing(e.to_string()))?;
            if auth.sequence == 0 {
                return Err(ClientError::Signing(
                    "sequence payments must set the account Sequence".to_owned(),
                ));
            }
            obj.insert("Sequence".to_owned(), json!(auth.sequence));
            let _ = obj.remove("TicketSequence");
        }
        XrplAssetTransferMethod::TicketSequence => {
            let ticket = next_ticket(
                signer,
                rpc,
                signer.classic_address(),
                options,
                requirements.max_timeout_seconds,
            )
            .await?;
            obj.insert("Sequence".to_owned(), json!(0));
            obj.insert("TicketSequence".to_owned(), json!(ticket));
        }
    }
    Ok(())
}

async fn next_ticket<R: XrplRpc>(
    signer: &XrplSigner,
    rpc: &R,
    account: &str,
    options: &XrplClientOptions,
    max_timeout_seconds: u64,
) -> Result<u32, ClientError> {
    let existing = rpc
        .ticket_sequences(account)
        .await
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    if let Some(ticket) = existing.first().copied() {
        return Ok(ticket);
    }
    let count = options.ticket_create_count.unwrap_or(1);
    if count == 0 {
        return Err(ClientError::Signing(format!(
            "No available XRPL ticket for {account}; automatic ticket creation is disabled"
        )));
    }
    let created = create_tickets(signer, rpc, count, max_timeout_seconds).await?;
    created.first().copied().ok_or_else(|| {
        ClientError::Signing(format!("TicketCreate returned no tickets for {account}"))
    })
}

/// Creates tickets for `signer` and returns the created sequences.
///
/// # Errors
///
/// Returns [`ClientError`] when `ticket_count` is out of range, RPC fails, or
/// `TicketCreate` does not validate as `tesSUCCESS`.
pub async fn create_tickets<R: XrplRpc>(
    signer: &XrplSigner,
    rpc: &R,
    ticket_count: u32,
    max_timeout_seconds: u64,
) -> Result<Vec<u32>, ClientError> {
    if !(1..=MAX_ACCOUNT_TICKETS).contains(&ticket_count) {
        return Err(ClientError::Signing(format!(
            "ticketCount must be an integer between 1 and {MAX_ACCOUNT_TICKETS}"
        )));
    }
    let current = rpc
        .current_ledger_index()
        .await
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let last_ledger = max_last_ledger_sequence(current, max_timeout_seconds);
    let fee = rpc
        .fee_drops()
        .await
        .map_err(|e| ClientError::Signing(e.to_string()))?
        .min(DEFAULT_MAX_FEE_DROPS);
    let auth = rpc
        .account_authorization(&signer.classic_address)
        .await
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let mut tx = json!({
        "TransactionType": "TicketCreate",
        "Account": signer.classic_address,
        "TicketCount": ticket_count,
        "Sequence": auth.sequence,
        "Fee": fee.to_string(),
        "LastLedgerSequence": last_ledger,
    });
    let blob = sign_transaction(&mut tx, &signer.private_key, &signer.public_key)
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let hash = signed_tx_hash(&blob).map_err(|e| ClientError::Signing(e.to_string()))?;
    let settled = submit_and_wait(rpc, &blob, &hash, last_ledger)
        .await
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    if !settled.validated || settled.result_code != "tesSUCCESS" {
        return Err(ClientError::Signing(format!(
            "TicketCreate failed: {}",
            settled.result_code
        )));
    }
    let meta = settled.meta.ok_or_else(|| {
        ClientError::Signing("TicketCreate returned no transaction metadata".to_owned())
    })?;
    Ok(extract_created_ticket_sequences(&meta))
}

/// XRPL exact scheme client for building and signing payment payloads.
#[derive(Clone)]
pub struct XrplExactClient<S, R> {
    signer: S,
    rpc: R,
    options: XrplClientOptions,
}

impl<S, R> std::fmt::Debug for XrplExactClient<S, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XrplExactClient").finish_non_exhaustive()
    }
}

impl<S, R> XrplExactClient<S, R> {
    /// Creates a new XRPL exact client.
    pub const fn new(signer: S, rpc: R) -> Self {
        Self {
            signer,
            rpc,
            options: XrplClientOptions {
                fee_drops: None,
                ticket_create_count: None,
            },
        }
    }

    /// Overrides fee and ticket-create options.
    #[must_use]
    pub fn with_options(mut self, options: XrplClientOptions) -> Self {
        self.options = options;
        self
    }
}

impl<S, R> SchemeId for XrplExactClient<S, R> {
    fn namespace(&self) -> &str {
        XrplExact.namespace()
    }

    fn scheme(&self) -> &str {
        XrplExact.scheme()
    }
}

impl<S, R> r402_core::scheme::Sealed for XrplExactClient<S, R> {}

impl<S, R> SchemeClient for XrplExactClient<S, R>
where
    S: AsRef<XrplSigner> + Send + Sync + Clone + 'static,
    R: XrplRpc + Clone + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: types::v2::PaymentRequirements = v.as_concrete()?;
                let chain_id = requirements.network.clone();
                if chain_id.namespace() != "xrpl" {
                    return None;
                }
                let _ = XrplChainReference::try_from(chain_id.clone()).ok()?;
                Some(PaymentCandidate {
                    chain_id,
                    asset: requirements.asset.to_string().into(),
                    amount: requirements.amount.as_str().into(),
                    scheme: self.scheme().into(),
                    pay_to: requirements.pay_to.to_string().into(),
                    requirements: v.clone(),
                    signer: Box::new(V2PayloadSigner {
                        signer: self.signer.clone(),
                        rpc: self.rpc.clone(),
                        options: self.options.clone(),
                        requirements,
                        resource: payment_required.resource.clone(),
                    }),
                })
            })
            .collect()
    }

    fn find_default_asset(&self, asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
        crate::find_default_xrpl_asset(asset, network)
    }
}

impl AsRef<Self> for XrplSigner {
    fn as_ref(&self) -> &Self {
        self
    }
}

struct V2PayloadSigner<S, R> {
    signer: S,
    rpc: R,
    options: XrplClientOptions,
    requirements: types::v2::PaymentRequirements,
    resource: ResourceInfo,
}

impl<S, R> PaymentCandidateSigner for V2PayloadSigner<S, R>
where
    S: AsRef<XrplSigner> + Send + Sync,
    R: XrplRpc,
{
    fn sign_payment(&self) -> r402_core::facilitator::BoxFuture<'_, Result<String, ClientError>> {
        Box::pin(async move {
            let blob = create_signed_payment(
                self.signer.as_ref(),
                &self.rpc,
                &self.requirements,
                &self.options,
            )
            .await?;
            let payload = types::v2::PaymentPayload::new(
                self.requirements.clone(),
                ExactXrplPayload {
                    signed_tx_blob: blob,
                },
            )
            .with_resource(self.resource.clone());
            let json = serde_json::to_vec(&payload)?;
            let encoded = Base64Bytes::encode(&json);
            Ok(encoded.to_string())
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unused_async,
    reason = "test assertions"
)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;
    use crate::chain::rpc::{
        XrplAccountAuthorization, XrplRpcError, XrplSimulationResult, XrplSubmitResult,
        XrplTxResult,
    };

    #[test]
    fn from_seed_derives_classic_address() {
        let signer = XrplSigner::from_seed("sEdTM1uX8pu2do5XvTnutH6HsouMaM2");
        assert!(signer.is_ok(), "{signer:?}");
        let signer = signer.unwrap();
        assert!(signer.classic_address().starts_with('r'));
        assert!(
            signer.public_key().starts_with("ED")
                || signer.public_key().starts_with("02")
                || signer.public_key().starts_with("03")
        );
    }

    struct TicketRpc {
        lookups: AtomicU32,
    }

    impl XrplRpc for TicketRpc {
        async fn current_ledger_index(&self) -> Result<u32, XrplRpcError> {
            Ok(990)
        }

        async fn account_authorization(
            &self,
            _account: &str,
        ) -> Result<XrplAccountAuthorization, XrplRpcError> {
            Ok(XrplAccountAuthorization {
                regular_key: None,
                is_master_key_disabled: false,
                sequence: 1,
            })
        }

        async fn ticket_sequences(&self, _account: &str) -> Result<Vec<u32>, XrplRpcError> {
            Ok(Vec::new())
        }

        async fn fee_drops(&self) -> Result<u64, XrplRpcError> {
            Ok(12)
        }

        async fn simulate(
            &self,
            _unsigned_tx: &Value,
        ) -> Result<XrplSimulationResult, XrplRpcError> {
            Ok(XrplSimulationResult {
                engine_result: "tesSUCCESS".to_owned(),
                engine_result_message: None,
            })
        }

        async fn submit(&self, _signed_tx_blob: &str) -> Result<XrplSubmitResult, XrplRpcError> {
            Ok(XrplSubmitResult {
                hash: "AB".repeat(32),
                engine_result: "tesSUCCESS".to_owned(),
            })
        }

        async fn tx(&self, hash: &str) -> Result<Option<XrplTxResult>, XrplRpcError> {
            let n = self.lookups.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                return Ok(None);
            }
            Ok(Some(XrplTxResult {
                hash: hash.to_owned(),
                validated: true,
                result_code: "tesSUCCESS".to_owned(),
                meta: Some(json!({
                    "TransactionResult": "tesSUCCESS",
                    "AffectedNodes": [{
                        "CreatedNode": {
                            "LedgerEntryType": "Ticket",
                            "NewFields": { "TicketSequence": 8 }
                        }
                    }]
                })),
            }))
        }
    }

    #[tokio::test]
    async fn create_tickets_waits_for_validated_result() {
        let signer = XrplSigner::from_seed("sEdTM1uX8pu2do5XvTnutH6HsouMaM2").unwrap();
        let rpc = TicketRpc {
            lookups: AtomicU32::new(0),
        };
        let tickets = create_tickets(&signer, &rpc, 1, 60).await.unwrap();
        assert_eq!(tickets, vec![8]);
        assert!(rpc.lookups.load(Ordering::SeqCst) >= 2);
    }
}
