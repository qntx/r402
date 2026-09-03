//! Concordium node queries. Live impl uses gRPC v2; tests supply a mock.

use std::fmt::{Debug, Formatter};
use std::future::Future;
use std::time::Duration;

use concordium_rust_sdk::base::contracts_common::AccountAddress;
use concordium_rust_sdk::base::protocol_level_tokens::TokenId;
use concordium_rust_sdk::base::transactions::AccountTransactionV1;
use concordium_rust_sdk::types::AccountInfo;
use concordium_rust_sdk::types::hashes::TransactionHash;
use concordium_rust_sdk::types::transactions::{BlockItem, EncodedPayload};
use concordium_rust_sdk::v2::{BlockIdentifier, Client, Endpoint};

use crate::exact::payload::{TransactionInfo, TransactionStatus};

/// Errors from node queries.
#[derive(Debug, thiserror::Error)]
pub enum ConcordiumRpcError {
    /// Transport or protocol failure.
    #[error("concordium rpc error: {0}")]
    Transport(String),
    /// Response could not be interpreted.
    #[error("concordium rpc parse error: {0}")]
    Parse(String),
}

/// On-chain account fields used by verify preflight and signature checks.
#[derive(Debug, Clone)]
pub struct AccountSnapshot {
    /// Next nonce, when the node supplied it.
    pub nonce: Option<u64>,
    /// Unencrypted balance in microCCD, when the node supplied it.
    pub amount_micro_ccd: Option<u64>,
    /// Full account info when the node supplied it (signature verification).
    pub info: Option<AccountInfo>,
    /// Ed25519 seed at credential 0 / key 0 when `info` is absent (mocks).
    pub key_seed: Option<[u8; 32]>,
}

/// Node operations required by the exact client and facilitator.
pub trait ConcordiumNode: Send + Sync {
    /// Account nonce, balance, and optional credential map.
    fn get_account(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<AccountSnapshot, ConcordiumRpcError>> + Send;

    /// Next account nonce.
    fn next_nonce(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<u64, ConcordiumRpcError>> + Send;

    /// On-chain PLT decimals.
    fn get_token_decimals(
        &self,
        token_id: &str,
    ) -> impl Future<Output = Result<u8, ConcordiumRpcError>> + Send;

    /// PLT balance in smallest units, or `None` when the account has no row.
    fn get_token_balance(
        &self,
        address: &str,
        token_id: &str,
    ) -> impl Future<Output = Result<Option<u128>, ConcordiumRpcError>> + Send;

    /// Broadcast a fully-signed V1 transaction. Returns hex hash.
    fn send_v1(
        &self,
        tx: AccountTransactionV1<EncodedPayload>,
    ) -> impl Future<Output = Result<String, ConcordiumRpcError>> + Send;

    /// Wait for `ConcordiumBFT` finalization.
    fn wait_finalized(
        &self,
        tx_hash: &str,
        timeout: Duration,
    ) -> impl Future<Output = Result<TransactionInfo, ConcordiumRpcError>> + Send;
}

/// Live gRPC v2 client.
#[derive(Clone)]
pub struct ConcordiumGrpc {
    client: Client,
}

impl Debug for ConcordiumGrpc {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConcordiumGrpc").finish_non_exhaustive()
    }
}

impl ConcordiumGrpc {
    /// Connects to a Concordium node.
    ///
    /// `endpoint` is a URI (`https://grpc.testnet.concordium.com:20000`) or a
    /// `host:port` pair from [`super::account::get_concordium_grpc_url`].
    ///
    /// # Errors
    ///
    /// Invalid URI or gRPC connect failure.
    pub async fn connect(endpoint: impl AsRef<str>) -> Result<Self, ConcordiumRpcError> {
        let uri = normalize_endpoint(endpoint.as_ref());
        let ep =
            Endpoint::from_shared(uri).map_err(|e| ConcordiumRpcError::Transport(e.to_string()))?;
        let client = Client::new(ep)
            .await
            .map_err(|e| ConcordiumRpcError::Transport(e.to_string()))?;
        Ok(Self { client })
    }
}

fn normalize_endpoint(raw: &str) -> String {
    if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    }
}

fn parse_account(address: &str) -> Result<AccountAddress, ConcordiumRpcError> {
    address
        .parse()
        .map_err(|e| ConcordiumRpcError::Parse(format!("{e}")))
}

#[allow(
    clippy::significant_drop_tightening,
    reason = "tonic Client clone is used across the await"
)]
impl ConcordiumNode for ConcordiumGrpc {
    async fn get_account(&self, address: &str) -> Result<AccountSnapshot, ConcordiumRpcError> {
        let mut client = self.client.clone();
        let addr = parse_account(address)?;
        let info = client
            .get_account_info(&addr.into(), BlockIdentifier::LastFinal)
            .await
            .map_err(|e| ConcordiumRpcError::Transport(e.to_string()))?
            .response;
        Ok(AccountSnapshot {
            nonce: Some(info.account_nonce.nonce),
            amount_micro_ccd: Some(info.account_amount.micro_ccd()),
            info: Some(info),
            key_seed: None,
        })
    }

    async fn next_nonce(&self, address: &str) -> Result<u64, ConcordiumRpcError> {
        let mut client = self.client.clone();
        let addr = parse_account(address)?;
        let nonce = client
            .get_next_account_sequence_number(&addr)
            .await
            .map_err(|e| ConcordiumRpcError::Transport(e.to_string()))?
            .nonce
            .nonce;
        Ok(nonce)
    }

    async fn get_token_decimals(&self, token_id: &str) -> Result<u8, ConcordiumRpcError> {
        let mut client = self.client.clone();
        let id: TokenId = token_id
            .parse()
            .map_err(|e| ConcordiumRpcError::Parse(format!("{e}")))?;
        let info = client
            .get_token_info(id, BlockIdentifier::LastFinal)
            .await
            .map_err(|e| ConcordiumRpcError::Transport(e.to_string()))?
            .response;
        Ok(info.token_state.decimals)
    }

    async fn get_token_balance(
        &self,
        address: &str,
        token_id: &str,
    ) -> Result<Option<u128>, ConcordiumRpcError> {
        let snapshot = self.get_account(address).await?;
        let Some(info) = snapshot.info else {
            return Ok(None);
        };
        let id: TokenId = token_id
            .parse()
            .map_err(|e| ConcordiumRpcError::Parse(format!("{e}")))?;
        Ok(info.token_amount(&id).map(|a| u128::from(a.value())))
    }

    async fn send_v1(
        &self,
        tx: AccountTransactionV1<EncodedPayload>,
    ) -> Result<String, ConcordiumRpcError> {
        let mut client = self.client.clone();
        let item = BlockItem::AccountTransactionV1(tx);
        let hash = client
            .send_block_item(&item)
            .await
            .map_err(|e| ConcordiumRpcError::Transport(e.to_string()))?;
        Ok(hash.to_string())
    }

    async fn wait_finalized(
        &self,
        tx_hash: &str,
        timeout: Duration,
    ) -> Result<TransactionInfo, ConcordiumRpcError> {
        let mut client = self.client.clone();
        let hash: TransactionHash = tx_hash
            .parse()
            .map_err(|e| ConcordiumRpcError::Parse(format!("{e}")))?;
        let outcome = tokio::time::timeout(timeout, client.wait_until_finalized(&hash))
            .await
            .map_err(|_| ConcordiumRpcError::Transport("finalization timeout".to_owned()))?
            .map_err(|e| ConcordiumRpcError::Transport(e.to_string()))?;
        let summary = outcome.1;
        parse_block_summary(tx_hash, &summary)
    }
}

fn parse_block_summary(
    tx_hash: &str,
    summary: &concordium_rust_sdk::types::BlockItemSummary,
) -> Result<TransactionInfo, ConcordiumRpcError> {
    use concordium_rust_sdk::types::{AccountTransactionEffects, BlockItemSummaryDetails};
    use concordium_rust_sdk::v2::Upward;

    if matches!(summary.is_reject(), Upward::Known(true)) {
        return Err(ConcordiumRpcError::Transport(format!(
            "Transaction {tx_hash} failed on-chain"
        )));
    }
    let sender = summary
        .sender_account()
        .map(|a| a.to_string())
        .unwrap_or_default();
    let (recipient, amount, asset) = match summary.details.as_ref() {
        Upward::Known(BlockItemSummaryDetails::AccountTransaction(details)) => {
            match details.effects.as_ref() {
                Upward::Known(
                    AccountTransactionEffects::AccountTransfer { amount, to, .. }
                    | AccountTransactionEffects::AccountTransferWithMemo { amount, to, .. },
                ) => (
                    Some(to.to_string()),
                    Some(amount.micro_ccd().to_string()),
                    Some("CCD".to_owned()),
                ),
                Upward::Known(AccountTransactionEffects::TokenUpdate { events }) => {
                    parse_token_events(events)
                }
                _ => (None, None, None),
            }
        }
        _ => (None, None, None),
    };
    Ok(TransactionInfo {
        tx_hash: tx_hash.to_owned(),
        status: TransactionStatus::Finalized,
        sender,
        recipient,
        amount,
        asset,
    })
}

fn parse_token_events(
    events: &[concordium_rust_sdk::base::protocol_level_tokens::TokenEvent],
) -> (Option<String>, Option<String>, Option<String>) {
    use concordium_rust_sdk::base::protocol_level_tokens::{TokenEventDetails, TokenHolder};
    for event in events {
        if let TokenEventDetails::Transfer(transfer) = &event.event {
            let recipient = match &transfer.to {
                TokenHolder::Account { address } => address.to_string(),
            };
            return (
                Some(recipient),
                Some(transfer.amount.value().to_string()),
                Some(event.token_id.as_ref().to_owned()),
            );
        }
    }
    (None, None, None)
}
