//! Facilitator-side TON chain provider: Highload V3 wallet + REST.

use std::fmt::{Debug, Formatter};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signer, SigningKey};
use r402_core::chain::{ChainId, ChainProvider};
use tonlib_core::cell::{BagOfCells, Cell};

use super::rpc::{TvmAccountState, TvmJettonWalletData, TvmProviderKind, TvmRpc, TvmRpcError};
use super::types::TvmChainReference;
use crate::chain::TvmAddress;
use crate::codecs::common::{BocError, encode_base64_boc};
use crate::codecs::highload_v3::{
    MAX_USABLE_QUERY_SEQNO, build_highload_inner, build_highload_v3_state_init,
    load_highload_query_state, pack_highload_external_body, query_id_is_processed,
    seqno_to_query_id, serialize_internal_transfer,
};
use crate::codecs::w5::{
    StateInitCells, address_from_state_init, build_external_message, build_internal_message,
    serialize_out_list, serialize_send_msg_action,
};
use crate::provider::TvmRestClient;
use crate::{
    DEFAULT_HIGHLOAD_SUBWALLET_ID, DEFAULT_HIGHLOAD_TIMEOUT, DEFAULT_RELAY_AMOUNT,
    DEFAULT_SETTLEMENT_BATCH_MAX_SIZE, DEFAULT_TONCENTER_EMULATION_TIMEOUT_SECONDS,
    DEFAULT_TONCENTER_TIMEOUT_SECONDS, DEFAULT_TRACE_CONFIRMATION_TIMEOUT_SECONDS,
    HIGHLOAD_V3_CODE_HASH, SEND_MODE_IGNORE_ERRORS, SEND_MODE_PAY_FEES_SEPARATELY,
};

/// Relay request wrapping a client-signed W5 body.
#[derive(Debug, Clone)]
pub struct TvmRelayRequest {
    /// Destination (payer W5 wallet).
    pub destination: TvmAddress,
    /// Signed W5 body.
    pub body: Cell,
    /// Optional payer `stateInit`.
    pub state_init: Option<StateInitCells>,
    /// TEP-74 `forward_ton_amount`.
    pub forward_ton_amount: u128,
    /// Computed outer relay amount (nanotons). `None` uses the wallet default.
    pub relay_amount: Option<u128>,
}

/// Highload V3 facilitator wallet configuration.
#[derive(Clone)]
pub struct HighloadV3Config {
    signing_key: SigningKey,
    /// Optional REST API key.
    pub api_key: Option<String>,
    /// Highload subwallet id.
    pub subwallet_id: u32,
    /// Highload timeout seconds.
    pub timeout: u32,
    /// Default nanotons attached per relayed inner message.
    pub relay_amount: u128,
    /// Optional REST base URL override.
    pub provider_base_url: Option<String>,
    /// REST timeout seconds.
    pub provider_timeout_seconds: u64,
    /// Emulation timeout seconds.
    pub provider_emulation_timeout_seconds: u64,
    /// Wallet workchain.
    pub workchain: i32,
    /// REST provider.
    pub provider: TvmProviderKind,
}

impl Debug for HighloadV3Config {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HighloadV3Config")
            .field("subwallet_id", &self.subwallet_id)
            .field("timeout", &self.timeout)
            .field("workchain", &self.workchain)
            .finish_non_exhaustive()
    }
}

impl HighloadV3Config {
    /// Constructs a config from a 32-byte seed or 64-byte secret key.
    ///
    /// # Errors
    ///
    /// Returns [`TvmRpcError::Parse`] if the key length is not 32 or 64.
    pub fn from_private_key(private_key: &[u8]) -> Result<Self, TvmRpcError> {
        let signing_key = signing_key_from_bytes(private_key)?;
        Ok(Self {
            signing_key,
            api_key: None,
            subwallet_id: DEFAULT_HIGHLOAD_SUBWALLET_ID,
            timeout: DEFAULT_HIGHLOAD_TIMEOUT,
            relay_amount: DEFAULT_RELAY_AMOUNT,
            provider_base_url: None,
            provider_timeout_seconds: DEFAULT_TONCENTER_TIMEOUT_SECONDS,
            provider_emulation_timeout_seconds: DEFAULT_TONCENTER_EMULATION_TIMEOUT_SECONDS,
            workchain: 0,
            provider: TvmProviderKind::Toncenter,
        })
    }

    /// Parses hex or base64 private key material.
    ///
    /// # Errors
    ///
    /// Returns [`TvmRpcError::Parse`] if decoding or key length fails.
    pub fn from_private_key_str(private_key: &str) -> Result<Self, TvmRpcError> {
        let bytes = parse_private_key_bytes(private_key)?;
        Self::from_private_key(&bytes)
    }

    /// Ed25519 public key.
    #[must_use]
    pub fn public_key(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }
}

/// Provider for interacting with a TON blockchain as a facilitator.
#[derive(Clone)]
pub struct TvmChainProvider {
    chain: TvmChainReference,
    config: HighloadV3Config,
    state_init: StateInitCells,
    address: TvmAddress,
    rpc: TvmRestClient,
    query_seqno: Arc<Mutex<u32>>,
    deployed: Arc<Mutex<Option<bool>>>,
}

impl Debug for TvmChainProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TvmChainProvider")
            .field("chain", &self.chain)
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

impl TvmChainProvider {
    /// Creates a provider for `chain` using `config`.
    ///
    /// # Errors
    ///
    /// Returns [`TvmRpcError`] if Highload construction or REST setup fails.
    pub fn new(chain: TvmChainReference, config: HighloadV3Config) -> Result<Self, TvmRpcError> {
        let public_key = config.public_key();
        let state_init =
            build_highload_v3_state_init(&public_key, config.subwallet_id, config.timeout)
                .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
        let address = address_from_state_init(&state_init, config.workchain)
            .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
        let rpc = TvmRestClient::connect(
            chain,
            config.provider,
            config.api_key.as_deref(),
            config.provider_base_url.as_deref(),
            Some(config.provider_timeout_seconds),
        )?;
        let initial_seqno = initial_query_seqno();
        Ok(Self {
            chain,
            config,
            state_init,
            address,
            rpc,
            query_seqno: Arc::new(Mutex::new(initial_seqno)),
            deployed: Arc::new(Mutex::new(None)),
        })
    }

    /// Highload V3 wallet address (raw).
    #[must_use]
    pub const fn address(&self) -> &TvmAddress {
        &self.address
    }

    /// Network this provider is bound to.
    #[must_use]
    pub const fn chain_reference(&self) -> TvmChainReference {
        self.chain
    }

    /// Shared REST handle.
    #[must_use]
    pub const fn rpc(&self) -> &TvmRestClient {
        &self.rpc
    }

    /// Builds a Highload V3 external BoC for `relay_requests`.
    ///
    /// # Errors
    ///
    /// Returns [`TvmRpcError`] if construction or account lookup fails.
    pub async fn build_relay_external_boc_batch(
        &self,
        relay_requests: &[TvmRelayRequest],
        for_emulation: bool,
    ) -> Result<Vec<u8>, TvmRpcError> {
        if relay_requests.is_empty() {
            return Err(TvmRpcError::Parse(
                "relayRequests must not be empty".to_owned(),
            ));
        }
        if relay_requests.len() > DEFAULT_SETTLEMENT_BATCH_MAX_SIZE {
            return Err(TvmRpcError::Parse(format!(
                "relayRequests must not exceed {DEFAULT_SETTLEMENT_BATCH_MAX_SIZE}"
            )));
        }
        let query_id = self.select_query_id(for_emulation).await?;
        let created_at = now_unix().saturating_sub(5);
        let mut forward_actions = Vec::with_capacity(relay_requests.len());
        for relay in relay_requests {
            let dest = relay
                .destination
                .to_ton()
                .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
            let forward_value = relay.relay_amount.unwrap_or(
                self.config
                    .relay_amount
                    .saturating_add(relay.forward_ton_amount),
            );
            let forward = build_internal_message(
                &dest,
                forward_value,
                true,
                relay.state_init.as_ref(),
                &relay.body,
            )
            .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
            let action = serialize_send_msg_action(
                &forward,
                SEND_MODE_PAY_FEES_SEPARATELY + SEND_MODE_IGNORE_ERRORS,
            )
            .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
            forward_actions.push(action);
        }
        let message_to_send = self.pack_actions_message(&forward_actions, query_id)?;
        let inner = build_highload_inner(
            self.config.subwallet_id,
            &message_to_send,
            query_id,
            created_at,
            self.config.timeout,
        )
        .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
        let external_body =
            pack_highload_external_body(&self.sign(cell_hash_bytes(&inner)), &inner)
                .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
        let mut external_state_init = None;
        let needs_lookup = {
            let deployed = self
                .deployed
                .lock()
                .map_err(|_| TvmRpcError::Parse("deployed lock poisoned".to_owned()))?;
            *deployed != Some(true)
        };
        if needs_lookup {
            let account = self.rpc.get_account_state(self.address.as_str()).await?;
            let mut deployed = self
                .deployed
                .lock()
                .map_err(|_| TvmRpcError::Parse("deployed lock poisoned".to_owned()))?;
            *deployed = Some(account.is_active);
            if account.is_uninitialized {
                external_state_init = Some(self.state_init.clone());
            }
        }
        let dest = self
            .address
            .to_ton()
            .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
        let external = build_external_message(&dest, external_state_init.as_ref(), &external_body)
            .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
        BagOfCells::from_root(external)
            .serialize(true)
            .map_err(|e| TvmRpcError::Parse(e.to_string()))
    }

    /// Emulates `external_boc`.
    ///
    /// # Errors
    ///
    /// Returns [`TvmRpcError`] on REST failure.
    pub async fn emulate_external_message(
        &self,
        external_boc: &[u8],
    ) -> Result<serde_json::Value, TvmRpcError> {
        self.rpc
            .emulate_trace(
                external_boc,
                false,
                self.config.provider_emulation_timeout_seconds,
            )
            .await
    }

    /// Broadcasts `external_boc`.
    ///
    /// # Errors
    ///
    /// Returns [`TvmRpcError`] on REST failure.
    pub async fn send_external_message(&self, external_boc: &[u8]) -> Result<String, TvmRpcError> {
        self.rpc.send_message(external_boc).await
    }

    /// Polls until the trace is complete or `timeout_seconds` elapses.
    ///
    /// Incomplete traces and "not found" responses are retried. Other RPC
    /// errors fail immediately so they are not swallowed.
    ///
    /// # Errors
    ///
    /// Returns [`TvmRpcError`] on a non-retryable RPC error or timeout.
    pub async fn wait_for_trace_confirmation(
        &self,
        trace_external_hash_norm: &str,
        timeout_seconds: u64,
    ) -> Result<serde_json::Value, TvmRpcError> {
        let timeout = timeout_seconds.max(1);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout);
        let mut last_err: Option<TvmRpcError> = None;
        while tokio::time::Instant::now() < deadline {
            match self
                .rpc
                .get_trace_by_message_hash(trace_external_hash_norm)
                .await
            {
                Ok(trace) => {
                    if trace
                        .get("is_incomplete")
                        .and_then(serde_json::Value::as_bool)
                        != Some(true)
                    {
                        return Ok(trace);
                    }
                }
                Err(err) if is_retryable_trace_error(&err) => {
                    last_err = Some(err);
                }
                Err(err) => return Err(err),
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        Err(last_err.unwrap_or_else(|| {
            TvmRpcError::Rpc(format!(
                "Timed out waiting for complete trace {trace_external_hash_norm}"
            ))
        }))
    }

    /// Default confirmation timeout.
    #[must_use]
    pub const fn confirmation_timeout_seconds() -> u64 {
        DEFAULT_TRACE_CONFIRMATION_TIMEOUT_SECONDS
    }

    async fn select_query_id(&self, for_emulation: bool) -> Result<u32, TvmRpcError> {
        let account = self.rpc.get_account_state(self.address.as_str()).await?;
        let query_state = if account.is_active {
            if let Some(init) = &account.state_init {
                if crate::codecs::common::cell_hash_hex(&init.code) != HIGHLOAD_V3_CODE_HASH {
                    return Err(TvmRpcError::Parse(
                        "Unexpected code hash for Highload V3 facilitator wallet".to_owned(),
                    ));
                }
                Some(
                    load_highload_query_state(&init.data, now_unix())
                        .map_err(|e| TvmRpcError::Parse(e.to_string()))?,
                )
            } else {
                return Err(TvmRpcError::Parse(
                    "Active Highload V3 wallet state is missing code or data".to_owned(),
                ));
            }
        } else {
            None
        };
        {
            let mut deployed = self
                .deployed
                .lock()
                .map_err(|_| TvmRpcError::Parse("deployed lock poisoned".to_owned()))?;
            *deployed = Some(query_state.is_some());
        }
        let mut guard = self
            .query_seqno
            .lock()
            .map_err(|_| TvmRpcError::Parse("query_seqno lock poisoned".to_owned()))?;
        let (query_id, next) = take_free_query_id(*guard, query_state.as_ref())?;
        if !for_emulation {
            *guard = next;
        }
        Ok(query_id)
    }

    fn pack_actions_message(&self, actions: &[Cell], query_id: u32) -> Result<Cell, TvmRpcError> {
        let dest = self
            .address
            .to_ton()
            .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
        let out_list =
            serialize_out_list(actions).map_err(|e| TvmRpcError::Parse(e.to_string()))?;
        let body = serialize_internal_transfer(&out_list, query_id)
            .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
        build_internal_message(&dest, 1_000_000_000, true, None, &body)
            .map_err(|e| TvmRpcError::Parse(e.to_string()))
    }

    fn sign(&self, hash: [u8; 32]) -> [u8; 64] {
        self.config.signing_key.sign(&hash).to_bytes()
    }
}

impl TvmRpc for TvmChainProvider {
    fn get_account_state(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<TvmAccountState, TvmRpcError>> + Send {
        self.rpc.get_account_state(address)
    }

    fn get_jetton_wallet(
        &self,
        asset: &str,
        owner: &str,
    ) -> impl Future<Output = Result<TvmAddress, TvmRpcError>> + Send {
        self.rpc.get_jetton_wallet(asset, owner)
    }

    fn get_jetton_wallet_data(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<TvmJettonWalletData, TvmRpcError>> + Send {
        self.rpc.get_jetton_wallet_data(address)
    }

    fn emulate_trace(
        &self,
        boc: &[u8],
        ignore_chksig: bool,
        timeout_seconds: u64,
    ) -> impl Future<Output = Result<serde_json::Value, TvmRpcError>> + Send {
        self.rpc.emulate_trace(boc, ignore_chksig, timeout_seconds)
    }

    fn send_message(&self, boc: &[u8]) -> impl Future<Output = Result<String, TvmRpcError>> + Send {
        self.rpc.send_message(boc)
    }

    fn get_trace_by_message_hash(
        &self,
        message_hash: &str,
    ) -> impl Future<Output = Result<serde_json::Value, TvmRpcError>> + Send {
        self.rpc.get_trace_by_message_hash(message_hash)
    }
}

impl ChainProvider for TvmChainProvider {
    fn signer_addresses(&self) -> Vec<String> {
        vec![self.address.to_string()]
    }

    fn chain_id(&self) -> ChainId {
        self.chain.into()
    }
}

fn cell_hash_bytes(cell: &Cell) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(cell.cell_hash().as_slice());
    out
}

fn signing_key_from_bytes(private_key: &[u8]) -> Result<SigningKey, TvmRpcError> {
    match private_key.len() {
        32 => {
            let mut seed = [0u8; 32];
            seed.copy_from_slice(private_key);
            Ok(SigningKey::from_bytes(&seed))
        }
        64 => {
            let mut seed = [0u8; 32];
            let prefix = private_key
                .get(..32)
                .ok_or_else(|| TvmRpcError::Parse("TVM private key must be 64 bytes".to_owned()))?;
            seed.copy_from_slice(prefix);
            Ok(SigningKey::from_bytes(&seed))
        }
        _ => Err(TvmRpcError::Parse(
            "TVM private key must be 32 bytes (seed) or 64 bytes (secret key)".to_owned(),
        )),
    }
}

fn parse_private_key_bytes(private_key: &str) -> Result<Vec<u8>, TvmRpcError> {
    let mut value = private_key.trim();
    if let Some(stripped) = value.strip_prefix("0x") {
        value = stripped;
    }
    if value.len().is_multiple_of(2) && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        let mut out = vec![0u8; value.len() / 2];
        for (i, chunk) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
            let byte = u8::from_str_radix(
                std::str::from_utf8(chunk).map_err(|e| TvmRpcError::Parse(e.to_string()))?,
                16,
            )
            .map_err(|e| TvmRpcError::Parse(e.to_string()))?;
            if let Some(slot) = out.get_mut(i) {
                *slot = byte;
            }
        }
        return Ok(out);
    }
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, value)
        .map_err(|e| TvmRpcError::Parse(e.to_string()))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn initial_query_seqno() -> u32 {
    use rand::RngExt;
    rand::rng().random_range(0..=MAX_USABLE_QUERY_SEQNO)
}

/// Reserves the first unused Highload `query_id` starting at `start_seqno`.
///
/// Returns `(query_id, next_seqno)`. `next_seqno` is the cursor after this
/// reservation so concurrent callers cannot pick the same id.
pub(crate) fn take_free_query_id(
    start_seqno: u32,
    query_state: Option<&crate::codecs::highload_v3::HighloadQueryState>,
) -> Result<(u32, u32), TvmRpcError> {
    let mut next = start_seqno;
    for _ in 0..=MAX_USABLE_QUERY_SEQNO {
        let seqno = next;
        next = next.saturating_add(1) % (MAX_USABLE_QUERY_SEQNO + 1);
        let query_id = seqno_to_query_id(seqno).map_err(|e| TvmRpcError::Parse(e.to_string()))?;
        let used = query_state.is_some_and(|s| query_id_is_processed(s, query_id));
        if !used {
            return Ok((query_id, next));
        }
    }
    Err(TvmRpcError::Parse(
        "No free Highload V3 query_id available".to_owned(),
    ))
}

pub(crate) fn is_retryable_trace_error(err: &TvmRpcError) -> bool {
    let msg = err.to_string();
    if msg.contains("no trace") {
        return true;
    }
    parse_http_status(&msg)
        .is_some_and(|status| matches!(status, 404 | 429 | 500 | 502 | 503 | 504))
}

fn parse_http_status(msg: &str) -> Option<u16> {
    let after = msg.rsplit_once(": ")?.1;
    let digits: String = after
        .bytes()
        .take_while(u8::is_ascii_digit)
        .map(char::from)
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// Encodes a cell as base64 BoC.
pub fn cell_to_b64(cell: &Cell) -> Result<String, BocError> {
    encode_base64_boc(cell)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn take_free_query_id_advances_cursor() {
        let (first, next) = take_free_query_id(7, None).unwrap();
        assert_eq!(first, seqno_to_query_id(7).unwrap());
        let (second, _) = take_free_query_id(next, None).unwrap();
        assert_ne!(first, second);
        assert_eq!(second, seqno_to_query_id(8).unwrap());
    }

    #[test]
    fn trace_404_and_no_trace_are_retryable() {
        assert!(is_retryable_trace_error(&TvmRpcError::Parse(
            "Toncenter returned no trace for message hash abc".to_owned()
        )));
        assert!(is_retryable_trace_error(&TvmRpcError::Rpc(
            "/v2/traces/abc: 404 Not Found".to_owned()
        )));
        assert!(is_retryable_trace_error(&TvmRpcError::Rpc(
            "/api/v3/traces: 429 Too Many Requests".to_owned()
        )));
        assert!(is_retryable_trace_error(&TvmRpcError::Rpc(
            "/api/v3/traces: 503 Service Unavailable".to_owned()
        )));
        assert!(!is_retryable_trace_error(&TvmRpcError::Rpc(
            "/v2/traces/abc: 401 Unauthorized".to_owned()
        )));
        assert!(!is_retryable_trace_error(&TvmRpcError::Parse(
            "invalid traces response".to_owned()
        )));
        assert!(!is_retryable_trace_error(&TvmRpcError::Rpc(
            "account 0:abc balance 5 nanotons".to_owned()
        )));
    }
}
