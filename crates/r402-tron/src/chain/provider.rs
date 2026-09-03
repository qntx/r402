//! Tron chain provider backed by the `TronGrid` HTTP REST API.
//!
//! Unlike EVM chains, Tron does not expose a JSON-RPC interface compatible
//! with `alloy-provider`; instead, `TronGrid` (and any `TronGrid`-compatible
//! node) exposes a JSON-over-HTTP "wallet" API. This provider implements
//! the subset needed for x402 settlement:
//!
//! - `wallet/triggerconstantcontract` — read-only calls (e.g. `balanceOf`)
//! - `wallet/triggersmartcontract` — builds an unsigned state-changing call
//! - `wallet/broadcasttransaction` — submits a signed transaction
//! - `wallet/gettransactioninfobyid` — polls for confirmation
//!
//! Tron transaction signing is over the SHA256 hash of the serialized
//! `raw_data` protobuf, which `TronGrid` conveniently returns pre-computed as
//! the transaction's `txID` — so signing never requires protobuf encoding
//! on our side, only a secp256k1 signature over the returned digest.

use std::fmt::{Debug, Formatter};
use std::time::Duration;

use alloy_primitives::{Address as EvmAddress, Bytes, U256, hex};
use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::SolCall;
use r402_protocol::network::{ChainId, ChainProvider};
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

use crate::chain::{Address, TronChainReference};
use crate::exact::TronExactError;

/// An unsigned Tron transaction as returned by `triggersmartcontract`.
///
/// Opaque beyond its `tx_id` and `raw_data_hex`: this provider never
/// decodes the protobuf-encoded `raw_data`, it only signs over the
/// pre-computed digest (`tx_id`) and re-submits the untouched JSON value
/// with a `signature` field attached.
#[derive(Debug, Clone)]
pub struct UnsignedTransaction {
    /// Hex-encoded transaction ID (`SHA256(raw_data)`), the digest to sign.
    pub tx_id: [u8; 32],
    /// The full `transaction` JSON object returned by `TronGrid`, forwarded
    /// verbatim to `broadcasttransaction` with a `signature` array attached.
    raw: Value,
}

/// Outcome of `wallet/gettransactioninfobyid`.
#[derive(Debug, Clone, Deserialize)]
pub struct TransactionInfo {
    /// The transaction ID, hex-encoded without `0x` prefix.
    #[serde(default, rename = "id")]
    pub id: String,
    /// Execution receipt; `result` is `"SUCCESS"` on success.
    #[serde(default)]
    pub receipt: Option<TransactionReceipt>,
}

/// Execution receipt embedded in [`TransactionInfo`].
#[derive(Debug, Clone, Deserialize)]
pub struct TransactionReceipt {
    /// `"SUCCESS"` on success; any other value (e.g. `"REVERT"`) is a failure.
    #[serde(default)]
    pub result: Option<String>,
}

impl TransactionInfo {
    /// Returns `true` once `TronGrid` has indexed a receipt for this transaction.
    #[must_use]
    pub const fn is_confirmed(&self) -> bool {
        self.receipt.is_some()
    }

    /// Returns `true` if the receipt indicates successful execution.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.receipt
            .as_ref()
            .and_then(|r| r.result.as_deref())
            .is_some_and(|r| r == "SUCCESS")
    }
}

/// Thin HTTP client for the `TronGrid` "wallet" REST API.
pub struct TronGridClient {
    base_url: Url,
    http: reqwest::Client,
}

impl Debug for TronGridClient {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TronGridClient")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

fn is_loopback(url: &Url) -> bool {
    url.host_str()
        .is_some_and(|host| host == "127.0.0.1" || host == "localhost" || host == "::1")
}

impl TronGridClient {
    /// Creates a new client pointed at the given `TronGrid`-compatible base URL
    /// (e.g. `https://api.trongrid.io`).
    #[must_use]
    pub fn new(base_url: Url) -> Self {
        let mut builder = reqwest::Client::builder();
        if is_loopback(&base_url) {
            builder = builder.no_proxy();
        }
        let http = builder.build().unwrap_or_else(|_| reqwest::Client::new());
        Self { base_url, http }
    }

    /// Creates a client with a caller-supplied `reqwest::Client` (for
    /// sharing connection pools or attaching an API key header).
    #[must_use]
    pub const fn with_http_client(base_url: Url, http: reqwest::Client) -> Self {
        Self { base_url, http }
    }

    fn endpoint(&self, path: &str) -> Url {
        #[allow(
            clippy::expect_used,
            reason = "path is a hardcoded literal at every call site"
        )]
        self.base_url.join(path).expect("invalid TronGrid path")
    }

    async fn post_json(&self, path: &str, body: Value) -> Result<Value, TronExactError> {
        let response = self
            .http
            .post(self.endpoint(path))
            .json(&body)
            .send()
            .await
            .map_err(|e| TronExactError::TronGrid(e.to_string()))?;
        response
            .json::<Value>()
            .await
            .map_err(|e| TronExactError::TronGrid(e.to_string()))
    }

    /// Calls a read-only contract method via `wallet/triggerconstantcontract`.
    ///
    /// `calldata` is the full ABI-encoded call (selector + arguments); the
    /// return value is the raw ABI-encoded result bytes.
    ///
    /// # Errors
    ///
    /// Returns [`TronExactError::TronGrid`] on transport failure or if the
    /// node reports the call did not succeed.
    pub async fn trigger_constant_contract<C: SolCall + Sync>(
        &self,
        owner: EvmAddress,
        contract: EvmAddress,
        call: &C,
    ) -> Result<Bytes, TronExactError> {
        let (function_selector, parameter) = encode_trigger(call);
        let body = json!({
            "owner_address": format!("41{}", hex::encode(owner)),
            "contract_address": format!("41{}", hex::encode(contract)),
            "function_selector": function_selector,
            "parameter": parameter,
            "visible": false,
        });
        let response = self
            .post_json("wallet/triggerconstantcontract", body)
            .await?;
        let ok = response
            .get("result")
            .and_then(|r| r.get("result"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !ok {
            let message = response
                .get("result")
                .and_then(|r| r.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("triggerconstantcontract failed")
                .to_owned();
            return Err(TronExactError::TronGrid(message));
        }
        let hex_result = response
            .get("constant_result")
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(Value::as_str)
            .ok_or_else(|| TronExactError::TronGrid("missing constant_result".to_owned()))?;
        let bytes = hex::decode(hex_result)
            .map_err(|e| TronExactError::TronGrid(format!("invalid hex result: {e}")))?;
        Ok(Bytes::from(bytes))
    }

    /// Builds an unsigned state-changing contract call via
    /// `wallet/triggersmartcontract`.
    ///
    /// # Errors
    ///
    /// Returns [`TronExactError::TronGrid`] on transport failure or if the
    /// node rejects the call (e.g. insufficient energy/bandwidth estimate).
    pub async fn trigger_smart_contract<C: SolCall + Sync>(
        &self,
        owner: EvmAddress,
        contract: EvmAddress,
        call: &C,
        fee_limit: u64,
    ) -> Result<UnsignedTransaction, TronExactError> {
        let (function_selector, parameter) = encode_trigger(call);
        let body = json!({
            "owner_address": format!("41{}", hex::encode(owner)),
            "contract_address": format!("41{}", hex::encode(contract)),
            "function_selector": function_selector,
            "parameter": parameter,
            "fee_limit": fee_limit,
            "call_value": 0,
            "visible": false,
        });
        let response = self.post_json("wallet/triggersmartcontract", body).await?;
        let ok = response
            .get("result")
            .and_then(|r| r.get("result"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !ok {
            let message = response
                .get("result")
                .and_then(|r| r.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("triggersmartcontract failed")
                .to_owned();
            return Err(TronExactError::TronGrid(message));
        }
        let transaction = response
            .get("transaction")
            .cloned()
            .ok_or_else(|| TronExactError::TronGrid("missing transaction".to_owned()))?;
        let tx_id_hex = transaction
            .get("txID")
            .and_then(Value::as_str)
            .ok_or_else(|| TronExactError::TronGrid("missing txID".to_owned()))?;
        let tx_id_vec = hex::decode(tx_id_hex)
            .map_err(|e| TronExactError::TronGrid(format!("invalid txID: {e}")))?;
        let tx_id: [u8; 32] = tx_id_vec
            .try_into()
            .map_err(|_| TronExactError::TronGrid("txID is not 32 bytes".to_owned()))?;
        Ok(UnsignedTransaction {
            tx_id,
            raw: transaction,
        })
    }

    /// Broadcasts a signed transaction via `wallet/broadcasttransaction`.
    ///
    /// # Errors
    ///
    /// Returns [`TronExactError::TransactionFailed`] if the network rejects
    /// the transaction, or [`TronExactError::TronGrid`] on transport failure.
    pub async fn broadcast_transaction(
        &self,
        mut unsigned: UnsignedTransaction,
        signature: &[u8],
    ) -> Result<String, TronExactError> {
        let Value::Object(ref mut map) = unsigned.raw else {
            return Err(TronExactError::TronGrid(
                "malformed transaction envelope".to_owned(),
            ));
        };
        let _ = map.insert(
            "signature".to_owned(),
            Value::Array(vec![Value::String(hex::encode(signature))]),
        );
        let response = self
            .post_json("wallet/broadcasttransaction", unsigned.raw)
            .await?;
        let ok = response
            .get("result")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !ok {
            let message = response.get("message").and_then(Value::as_str).map_or_else(
                || "broadcasttransaction failed".to_owned(),
                |b64_or_text| {
                    // TronGrid often hex-encodes the message field.
                    hex::decode(b64_or_text)
                        .ok()
                        .and_then(|bytes| String::from_utf8(bytes).ok())
                        .unwrap_or_else(|| b64_or_text.to_owned())
                },
            );
            return Err(TronExactError::TransactionFailed(message));
        }
        Ok(hex::encode(unsigned.tx_id))
    }

    /// Polls `wallet/gettransactioninfobyid` once for the given transaction ID.
    ///
    /// # Errors
    ///
    /// Returns [`TronExactError::TronGrid`] on transport failure.
    pub async fn get_transaction_info(
        &self,
        tx_id_hex: &str,
    ) -> Result<TransactionInfo, TronExactError> {
        let body = json!({ "value": tx_id_hex });
        let response = self
            .post_json("wallet/gettransactioninfobyid", body)
            .await?;
        serde_json::from_value(response)
            .map_err(|e| TronExactError::TronGrid(format!("malformed transaction info: {e}")))
    }

    /// Polls until the transaction is confirmed or `timeout` elapses.
    ///
    /// # Errors
    ///
    /// Returns [`TronExactError::ConfirmationTimeout`] if no receipt appears
    /// within `timeout`, or [`TronExactError::TransactionFailed`] if the
    /// receipt indicates on-chain failure.
    pub async fn wait_for_confirmation(
        &self,
        tx_id_hex: &str,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<TransactionInfo, TronExactError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let info = self.get_transaction_info(tx_id_hex).await?;
            if info.is_confirmed() {
                return Self::confirm_result(tx_id_hex, info);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(TronExactError::ConfirmationTimeout);
            }
            tokio::time::sleep(poll_interval).await;
        }
    }

    fn confirm_result(
        tx_id_hex: &str,
        info: TransactionInfo,
    ) -> Result<TransactionInfo, TronExactError> {
        if info.is_success() {
            Ok(info)
        } else {
            Err(TronExactError::TransactionFailed(format!(
                "transaction {tx_id_hex} reverted"
            )))
        }
    }
}

/// java-tron `parseMethod` keccak256s `function_selector` as UTF-8 and takes
/// the first 4 bytes. The textual ABI signature (`balanceOf(address)`) hashes
/// to the real selector; hex of the 4-byte selector does not.
fn encode_trigger<C: SolCall>(call: &C) -> (&'static str, String) {
    let mut parameter = Vec::new();
    call.abi_encode_raw(&mut parameter);
    (C::SIGNATURE, hex::encode(parameter))
}

/// Configuration for constructing a [`TronChainProvider`].
#[derive(Debug, Clone)]
pub struct TronChainProviderConfig {
    /// The Tron network this provider operates on.
    pub chain_reference: TronChainReference,
    /// `TronGrid`-compatible base URL (e.g. `https://api.trongrid.io`).
    pub base_url: Url,
    /// The facilitator's signing key (pays energy/bandwidth for settlement).
    pub signer: PrivateKeySigner,
    /// Fee limit (in SUN) attached to settlement transactions.
    pub fee_limit: u64,
    /// How long to wait for transaction confirmation.
    pub confirmation_timeout: Duration,
    /// Interval between confirmation polls.
    pub confirmation_poll_interval: Duration,
}

/// Provider for interacting with the Tron blockchain via `TronGrid`.
///
/// Handles balance checks, transaction construction, signing, broadcast,
/// and confirmation polling for the Tron exact scheme facilitator.
pub struct TronChainProvider {
    chain_reference: TronChainReference,
    grid: TronGridClient,
    signer: PrivateKeySigner,
    fee_limit: u64,
    confirmation_timeout: Duration,
    confirmation_poll_interval: Duration,
}

impl Debug for TronChainProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TronChainProvider")
            .field("chain_reference", &self.chain_reference)
            .field("signer", &self.signer.address())
            .field("fee_limit", &self.fee_limit)
            .finish_non_exhaustive()
    }
}

impl TronChainProvider {
    /// Creates a new Tron chain provider from the given configuration.
    #[must_use]
    pub fn new(config: TronChainProviderConfig) -> Self {
        #[cfg(feature = "telemetry")]
        tracing::info!(
            chain = %ChainId::from(config.chain_reference),
            signer = %config.signer.address(),
            base_url = %config.base_url,
            "Using Tron provider"
        );
        Self {
            chain_reference: config.chain_reference,
            grid: TronGridClient::new(config.base_url),
            signer: config.signer,
            fee_limit: config.fee_limit,
            confirmation_timeout: config.confirmation_timeout,
            confirmation_poll_interval: config.confirmation_poll_interval,
        }
    }

    /// Returns the Tron network this provider operates on.
    #[must_use]
    pub const fn chain_reference(&self) -> TronChainReference {
        self.chain_reference
    }

    /// Returns the underlying `TronGrid` HTTP client.
    #[must_use]
    pub const fn grid(&self) -> &TronGridClient {
        &self.grid
    }

    /// Returns the facilitator's signing address (raw EVM hex form).
    #[must_use]
    pub const fn signer_address(&self) -> EvmAddress {
        self.signer.address()
    }

    /// Reads a TRC-20 token balance via `triggerconstantcontract`.
    ///
    /// # Errors
    ///
    /// Returns [`TronExactError::TronGrid`] on transport failure or a
    /// malformed response.
    pub async fn trc20_balance_of(
        &self,
        token: EvmAddress,
        account: EvmAddress,
    ) -> Result<U256, TronExactError> {
        let call = crate::chain::contracts::trc20::balanceOfCall { account };
        let result = self
            .grid
            .trigger_constant_contract(self.signer_address(), token, &call)
            .await?;
        let padded: [u8; 32] = result
            .get(..32)
            .and_then(|slice| slice.try_into().ok())
            .ok_or_else(|| TronExactError::TronGrid("malformed balanceOf result".to_owned()))?;
        Ok(U256::from_be_bytes(padded))
    }

    /// Builds, signs, broadcasts, and confirms a state-changing contract call.
    ///
    /// # Errors
    ///
    /// Returns a [`TronExactError`] variant on any step failure (request
    /// construction, signing, broadcast, or confirmation timeout/failure).
    pub async fn send_contract_call<C: SolCall + Sync>(
        &self,
        contract: EvmAddress,
        call: &C,
    ) -> Result<String, TronExactError> {
        let unsigned = self
            .grid
            .trigger_smart_contract(self.signer_address(), contract, call, self.fee_limit)
            .await?;
        let signature = self
            .signer
            .sign_hash(&unsigned.tx_id.into())
            .await
            .map_err(|e| TronExactError::SignatureRecovery(e.to_string()))?;
        let tx_id = self
            .grid
            .broadcast_transaction(unsigned, signature.as_bytes().as_ref())
            .await?;
        let info = self
            .grid
            .wait_for_confirmation(
                &tx_id,
                self.confirmation_timeout,
                self.confirmation_poll_interval,
            )
            .await?;
        Ok(info.id)
    }
}

impl ChainProvider for TronChainProvider {
    fn signer_addresses(&self) -> Vec<String> {
        vec![Address::from_evm(self.signer.address()).to_string()]
    }

    fn chain_id(&self) -> ChainId {
        self.chain_reference.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::contracts::trc20::balanceOfCall;

    #[test]
    fn encode_trigger_uses_textual_abi_signature() {
        let call = balanceOfCall {
            account: EvmAddress::ZERO,
        };
        let (selector, parameter) = encode_trigger(&call);
        assert_eq!(selector, "balanceOf(address)");
        assert_ne!(selector, hex::encode(balanceOfCall::SELECTOR));
        let mut raw = Vec::new();
        call.abi_encode_raw(&mut raw);
        assert_eq!(parameter, hex::encode(raw));
        assert!(!parameter.starts_with(&hex::encode(balanceOfCall::SELECTOR)));
    }

    #[tokio::test]
    async fn trigger_constant_contract_sends_textual_function_selector() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/wallet/triggerconstantcontract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "result": { "result": true },
                "constant_result": [
                    "00000000000000000000000000000000000000000000000000000000000f4240"
                ]
            })))
            .mount(&server)
            .await;

        let client = TronGridClient::new(Url::parse(&server.uri()).expect("mock url"));
        let call = balanceOfCall {
            account: EvmAddress::ZERO,
        };
        let result = client
            .trigger_constant_contract(EvmAddress::ZERO, EvmAddress::ZERO, &call)
            .await
            .expect("mock trigger");
        assert_eq!(result.len(), 32);

        let requests = server.received_requests().await.expect("recorded");
        let body: Value = serde_json::from_slice(
            &requests
                .first()
                .expect("one triggerconstantcontract request")
                .body,
        )
        .expect("json body");
        let sent_selector = body
            .get("function_selector")
            .and_then(Value::as_str)
            .expect("function_selector");
        assert_eq!(sent_selector, "balanceOf(address)");
        let selector_hex = hex::encode(balanceOfCall::SELECTOR);
        assert_ne!(sent_selector, selector_hex);
        let mut raw = Vec::new();
        call.abi_encode_raw(&mut raw);
        let parameter_hex = hex::encode(raw);
        assert_eq!(
            body.get("parameter").and_then(Value::as_str),
            Some(parameter_hex.as_str())
        );
    }
}
