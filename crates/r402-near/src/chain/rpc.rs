//! JSON-RPC client for NEAR preflight reads and (facilitator) settlement.

use std::fmt::{Debug, Formatter};

use near_jsonrpc_client::errors::JsonRpcError;
use near_jsonrpc_client::{JsonRpcClient, methods};
use near_jsonrpc_primitives::types::query::{QueryResponseKind, RpcQueryError};
use near_primitives::types::{AccountId, BlockReference, Finality, FunctionArgs};
use near_primitives::views::{
    AccessKeyPermissionView, ExecutionStatusView, FinalExecutionStatus, QueryRequest,
};
use serde::Deserialize;

use crate::EMPTY_CONTRACT_CODE_HASH;

/// Access-key permission variants relevant to delegate-action verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NearAccessKeyPermissionKind {
    /// Full-access key (required for `ft_transfer` with 1 yocto deposit).
    FullAccess,
    /// Restricted function-call key (must be rejected).
    FunctionCall,
    /// Any other permission variant (must be rejected).
    Unknown,
}

/// Result of an on-chain `view_access_key` query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NearAccessKeyView {
    /// On-chain nonce for the access key.
    pub nonce: u64,
    /// Normalized permission variant.
    pub permission_kind: NearAccessKeyPermissionKind,
}

/// Result of an on-chain `view_account` query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearAccountView {
    /// Base58 code hash; equals [`crate::EMPTY_CONTRACT_CODE_HASH`] when no
    /// contract is deployed.
    pub code_hash: String,
}

impl NearAccountView {
    /// Returns `true` when the account has no contract code.
    #[must_use]
    pub fn has_no_code(&self) -> bool {
        self.code_hash == EMPTY_CONTRACT_CODE_HASH
    }
}

/// Status of NEP-145 storage registration for a recipient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NearStorageBalance {
    /// Token does not implement NEP-145.
    Unsupported,
    /// Token implements NEP-145 and the account is registered.
    Registered,
    /// Token implements NEP-145 and the account is not registered.
    Unregistered,
}

/// Status of a single on-chain receipt outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NearReceiptStatus {
    /// Inner `ft_transfer` receipt succeeded.
    Success {
        /// Base64-encoded success value.
        value: String,
    },
    /// Inner receipt failed (or was not observed as success).
    Failure {
        /// Error description.
        error: String,
    },
}

/// Result of submitting the outer relayer transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearSettlementOutcome {
    /// Final outer transaction hash.
    pub transaction: String,
    /// Status of the inner `ft_transfer` receipt.
    pub inner_receipt: NearReceiptStatus,
}

/// Errors from NEAR JSON-RPC reads and submits.
#[derive(Debug, thiserror::Error)]
pub enum NearRpcError {
    /// JSON-RPC transport or handler failure.
    #[error("near rpc error: {0}")]
    Rpc(String),
    /// Response JSON could not be parsed.
    #[error("near rpc parse error: {0}")]
    Parse(String),
}

/// Read-only NEAR RPC operations used by the client and facilitator.
pub trait NearRpc: Send + Sync {
    /// Current final block height.
    fn current_block_height(&self) -> impl Future<Output = Result<u64, NearRpcError>> + Send;

    /// `view_account`. Resolves to `None` when the account does not exist.
    fn view_account(
        &self,
        account_id: &str,
    ) -> impl Future<Output = Result<Option<NearAccountView>, NearRpcError>> + Send;

    /// `view_access_key`. Resolves to `None` when the key does not exist.
    fn view_access_key(
        &self,
        account_id: &str,
        public_key: &str,
    ) -> impl Future<Output = Result<Option<NearAccessKeyView>, NearRpcError>> + Send;

    /// `ft_balance_of` on the token contract, in atomic units.
    fn ft_balance_of(
        &self,
        token: &str,
        account_id: &str,
    ) -> impl Future<Output = Result<u128, NearRpcError>> + Send;

    /// `storage_balance_of` on the token contract (NEP-145).
    fn storage_balance_of(
        &self,
        token: &str,
        account_id: &str,
    ) -> impl Future<Output = Result<NearStorageBalance, NearRpcError>> + Send;
}

/// JSON-RPC client pointed at a FastNEAR-compatible endpoint.
#[derive(Clone)]
pub struct NearJsonRpc {
    client: JsonRpcClient,
}

impl Debug for NearJsonRpc {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NearJsonRpc").finish_non_exhaustive()
    }
}

impl NearJsonRpc {
    /// Connects to the given RPC URL.
    #[must_use]
    pub fn connect(url: impl AsRef<str>) -> Self {
        Self {
            client: JsonRpcClient::connect(url.as_ref()),
        }
    }

    /// Returns a clone of the underlying JSON-RPC client.
    #[must_use]
    pub fn client(&self) -> JsonRpcClient {
        self.client.clone()
    }
}

impl NearRpc for NearJsonRpc {
    async fn current_block_height(&self) -> Result<u64, NearRpcError> {
        let block = self
            .client
            .call(methods::block::RpcBlockRequest {
                block_reference: BlockReference::Finality(Finality::Final),
            })
            .await
            .map_err(|e| NearRpcError::Rpc(e.to_string()))?;
        Ok(block.header.height)
    }

    async fn view_account(
        &self,
        account_id: &str,
    ) -> Result<Option<NearAccountView>, NearRpcError> {
        let account_id = account_id
            .parse::<AccountId>()
            .map_err(|e| NearRpcError::Parse(format!("{e}")))?;
        let request = methods::query::RpcQueryRequest {
            block_reference: BlockReference::Finality(Finality::Final),
            request: QueryRequest::ViewAccount { account_id },
        };
        match self.client.call(request).await {
            Ok(response) => match response.kind {
                QueryResponseKind::ViewAccount(view) => Ok(Some(NearAccountView {
                    code_hash: view.code_hash.to_string(),
                })),
                other => Err(NearRpcError::Parse(format!(
                    "unexpected view_account response: {other:?}"
                ))),
            },
            Err(err) if is_unknown_account(&err) => Ok(None),
            Err(err) => Err(NearRpcError::Rpc(err.to_string())),
        }
    }

    async fn view_access_key(
        &self,
        account_id: &str,
        public_key: &str,
    ) -> Result<Option<NearAccessKeyView>, NearRpcError> {
        let account_id = account_id
            .parse::<AccountId>()
            .map_err(|e| NearRpcError::Parse(format!("{e}")))?;
        let public_key = public_key
            .parse::<near_crypto::PublicKey>()
            .map_err(|e| NearRpcError::Parse(format!("{e}")))?;
        let request = methods::query::RpcQueryRequest {
            block_reference: BlockReference::Finality(Finality::Final),
            request: QueryRequest::ViewAccessKey {
                account_id,
                public_key,
            },
        };
        match self.client.call(request).await {
            Ok(response) => match response.kind {
                QueryResponseKind::AccessKey(view) => Ok(Some(NearAccessKeyView {
                    nonce: view.nonce,
                    permission_kind: permission_kind(&view.permission),
                })),
                other => Err(NearRpcError::Parse(format!(
                    "unexpected view_access_key response: {other:?}"
                ))),
            },
            Err(err) if is_unknown_access_key(&err) => Ok(None),
            Err(err) => Err(NearRpcError::Rpc(err.to_string())),
        }
    }

    async fn ft_balance_of(&self, token: &str, account_id: &str) -> Result<u128, NearRpcError> {
        let result = call_function(&self.client, token, "ft_balance_of", account_id).await?;
        parse_ft_balance(&result)
    }

    async fn storage_balance_of(
        &self,
        token: &str,
        account_id: &str,
    ) -> Result<NearStorageBalance, NearRpcError> {
        match call_function(&self.client, token, "storage_balance_of", account_id).await {
            Ok(bytes) => parse_storage_balance(&bytes),
            Err(err) if is_method_not_found_msg(&err.to_string()) => {
                Ok(NearStorageBalance::Unsupported)
            }
            Err(err) => Err(err),
        }
    }
}

impl<T: NearRpc + Send + Sync> NearRpc for &T {
    fn current_block_height(&self) -> impl Future<Output = Result<u64, NearRpcError>> + Send {
        (**self).current_block_height()
    }

    fn view_account(
        &self,
        account_id: &str,
    ) -> impl Future<Output = Result<Option<NearAccountView>, NearRpcError>> + Send {
        (**self).view_account(account_id)
    }

    fn view_access_key(
        &self,
        account_id: &str,
        public_key: &str,
    ) -> impl Future<Output = Result<Option<NearAccessKeyView>, NearRpcError>> + Send {
        (**self).view_access_key(account_id, public_key)
    }

    fn ft_balance_of(
        &self,
        token: &str,
        account_id: &str,
    ) -> impl Future<Output = Result<u128, NearRpcError>> + Send {
        (**self).ft_balance_of(token, account_id)
    }

    fn storage_balance_of(
        &self,
        token: &str,
        account_id: &str,
    ) -> impl Future<Output = Result<NearStorageBalance, NearRpcError>> + Send {
        (**self).storage_balance_of(token, account_id)
    }
}

impl<T: NearRpc + Send + Sync> NearRpc for std::sync::Arc<T> {
    fn current_block_height(&self) -> impl Future<Output = Result<u64, NearRpcError>> + Send {
        (**self).current_block_height()
    }

    fn view_account(
        &self,
        account_id: &str,
    ) -> impl Future<Output = Result<Option<NearAccountView>, NearRpcError>> + Send {
        (**self).view_account(account_id)
    }

    fn view_access_key(
        &self,
        account_id: &str,
        public_key: &str,
    ) -> impl Future<Output = Result<Option<NearAccessKeyView>, NearRpcError>> + Send {
        (**self).view_access_key(account_id, public_key)
    }

    fn ft_balance_of(
        &self,
        token: &str,
        account_id: &str,
    ) -> impl Future<Output = Result<u128, NearRpcError>> + Send {
        (**self).ft_balance_of(token, account_id)
    }

    fn storage_balance_of(
        &self,
        token: &str,
        account_id: &str,
    ) -> impl Future<Output = Result<NearStorageBalance, NearRpcError>> + Send {
        (**self).storage_balance_of(token, account_id)
    }
}

const fn permission_kind(permission: &AccessKeyPermissionView) -> NearAccessKeyPermissionKind {
    match permission {
        AccessKeyPermissionView::FullAccess => NearAccessKeyPermissionKind::FullAccess,
        AccessKeyPermissionView::FunctionCall { .. } => NearAccessKeyPermissionKind::FunctionCall,
        AccessKeyPermissionView::GasKeyFunctionCall { .. }
        | AccessKeyPermissionView::GasKeyFullAccess { .. } => NearAccessKeyPermissionKind::Unknown,
    }
}

fn is_unknown_account(err: &JsonRpcError<RpcQueryError>) -> bool {
    matches!(
        err.handler_error(),
        Some(RpcQueryError::UnknownAccount { .. })
    )
}

fn is_unknown_access_key(err: &JsonRpcError<RpcQueryError>) -> bool {
    matches!(
        err.handler_error(),
        Some(RpcQueryError::UnknownAccessKey { .. })
    )
}

fn is_method_not_found_msg(msg: &str) -> bool {
    msg.contains("MethodNotFound")
        || msg.contains("MethodResolveError")
        || msg.contains("MethodEmptyName")
}

async fn call_function(
    client: &JsonRpcClient,
    token: &str,
    method_name: &str,
    account_id: &str,
) -> Result<Vec<u8>, NearRpcError> {
    let contract_id = token
        .parse::<AccountId>()
        .map_err(|e| NearRpcError::Parse(format!("{e}")))?;
    let args = serde_json::to_vec(&serde_json::json!({ "account_id": account_id }))
        .map_err(|e| NearRpcError::Parse(e.to_string()))?;
    let request = methods::query::RpcQueryRequest {
        block_reference: BlockReference::Finality(Finality::Final),
        request: QueryRequest::CallFunction {
            account_id: contract_id,
            method_name: method_name.to_owned(),
            args: FunctionArgs::from(args),
        },
    };
    match client.call(request).await {
        Ok(response) => match response.kind {
            QueryResponseKind::CallResult(result) => Ok(result.result),
            other => Err(NearRpcError::Parse(format!(
                "unexpected {method_name} response: {other:?}"
            ))),
        },
        Err(err) => {
            if let Some(RpcQueryError::ContractExecutionError { vm_error, .. }) =
                err.handler_error()
                && is_method_not_found_msg(vm_error)
            {
                return Err(NearRpcError::Rpc(vm_error.clone()));
            }
            Err(NearRpcError::Rpc(err.to_string()))
        }
    }
}

fn parse_ft_balance(bytes: &[u8]) -> Result<u128, NearRpcError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| NearRpcError::Parse(e.to_string()))?;
    let as_str = match value {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    };
    let trimmed = as_str.trim_matches('"');
    if !trimmed.bytes().all(|b| b.is_ascii_digit()) {
        return Err(NearRpcError::Parse(format!(
            "invalid_ft_balance_of_result: {trimmed}"
        )));
    }
    trimmed
        .parse()
        .map_err(|_| NearRpcError::Parse("invalid_ft_balance_of_result".to_owned()))
}

fn parse_storage_balance(bytes: &[u8]) -> Result<NearStorageBalance, NearRpcError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| NearRpcError::Parse(e.to_string()))?;
    match value {
        serde_json::Value::Null => Ok(NearStorageBalance::Unregistered),
        serde_json::Value::Object(_) => Ok(NearStorageBalance::Registered),
        other => Err(NearRpcError::Parse(format!(
            "invalid storage_balance_of result: {other}"
        ))),
    }
}

/// Interprets a final execution outcome for settlement (spec §7).
///
/// Success requires a token-contract receipt with `SuccessValue`.
#[must_use]
pub fn interpret_settlement_outcome(
    status: &FinalExecutionStatus,
    receipts: &[(String, ExecutionStatusView)],
    token_contract_id: &str,
) -> NearReceiptStatus {
    if let FinalExecutionStatus::Failure(err) = status {
        return NearReceiptStatus::Failure {
            error: format!("{err:?}"),
        };
    }
    for (_executor, receipt_status) in receipts {
        if let ExecutionStatusView::Failure(err) = receipt_status {
            return NearReceiptStatus::Failure {
                error: format!("{err:?}"),
            };
        }
    }
    let token_status = receipts
        .iter()
        .find(|(executor, _)| executor == token_contract_id)
        .map(|(_, receipt_status)| receipt_status);
    match token_status {
        Some(ExecutionStatusView::SuccessValue(value)) => NearReceiptStatus::Success {
            value: String::from_utf8_lossy(value).into_owned(),
        },
        _ => NearReceiptStatus::Failure {
            error: "inner_ft_transfer_receipt_not_successful".to_owned(),
        },
    }
}

/// JSON shape of `ft_transfer` args.
#[derive(Debug, Clone, Deserialize)]
pub struct FtTransferArgs {
    /// Recipient account ID.
    pub receiver_id: String,
    /// Amount in atomic units as a decimal string.
    pub amount: String,
}

/// Parses NEP-141 `ft_transfer` JSON args.
///
/// # Errors
///
/// Returns [`NearRpcError::Parse`] when JSON is malformed or required fields
/// are missing.
pub fn parse_ft_transfer_args(args: &[u8]) -> Result<FtTransferArgs, NearRpcError> {
    let decoded: FtTransferArgs =
        serde_json::from_slice(args).map_err(|e| NearRpcError::Parse(e.to_string()))?;
    if decoded.receiver_id.is_empty() {
        return Err(NearRpcError::Parse(
            "invalid_ft_transfer_args_receiver_id".to_owned(),
        ));
    }
    if decoded.amount.is_empty() || !decoded.amount.bytes().all(|b| b.is_ascii_digit()) {
        return Err(NearRpcError::Parse(
            "invalid_ft_transfer_args_amount".to_owned(),
        ));
    }
    Ok(decoded)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn parse_ft_transfer_args_accepts_valid() {
        let args = br#"{"receiver_id":"merchant.testnet","amount":"1000000"}"#;
        let parsed = parse_ft_transfer_args(args).unwrap();
        assert_eq!(parsed.receiver_id, "merchant.testnet");
        assert_eq!(parsed.amount, "1000000");
    }

    #[test]
    fn parse_ft_transfer_args_rejects_bad_amount() {
        let args = br#"{"receiver_id":"merchant.testnet","amount":"1.0"}"#;
        assert!(parse_ft_transfer_args(args).is_err());
    }

    #[test]
    fn parse_storage_null_is_unregistered() {
        assert_eq!(
            parse_storage_balance(b"null").unwrap(),
            NearStorageBalance::Unregistered
        );
    }

    #[test]
    fn parse_storage_object_is_registered() {
        assert_eq!(
            parse_storage_balance(br#"{"total":"1","available":"1"}"#).unwrap(),
            NearStorageBalance::Registered
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]
mod json_rpc_tests {
    use serde_json::json;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    use super::*;

    struct QueryAccount;

    impl Respond for QueryAccount {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let body: serde_json::Value =
                serde_json::from_slice(&request.body).unwrap_or_else(|_| json!({}));
            let id = body.get("id").cloned().unwrap_or_else(|| json!("dontcare"));
            ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "amount": "0",
                    "locked": "0",
                    "code_hash": "11111111111111111111111111111112",
                    "storage_usage": 0,
                    "storage_paid_at": 0,
                    "block_height": 1000,
                    "block_hash": "11111111111111111111111111111111"
                }
            }))
        }
    }

    #[tokio::test]
    async fn view_account_via_json_rpc() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(QueryAccount)
            .mount(&server)
            .await;
        let rpc = NearJsonRpc::connect(server.uri());
        let account = rpc
            .view_account("alice.testnet")
            .await
            .expect("view_account")
            .expect("account exists");
        assert_eq!(account.code_hash, "11111111111111111111111111111112");
    }
}
