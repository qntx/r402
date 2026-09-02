//! Client-side payment signing for the TON `"exact"` scheme.

use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signer, SigningKey};
use r402_core::chain::ChainId;
use r402_core::error::ClientError;
use r402_core::scheme::SchemeId;
use r402_core::scheme::{DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use r402_core::wire::Base64Bytes;
use r402_core::wire::PaymentRequired;
use r402_core::wire::ResourceInfo;
use tonlib_core::TonAddress;
use tonlib_core::cell::Cell;

use crate::chain::rpc::TvmRpc;
use crate::chain::{TvmAddress, TvmChainReference};
use crate::codecs::common::{BocError, encode_base64_boc};
use crate::codecs::jetton::build_jetton_transfer_body;
use crate::codecs::w5::{
    StateInitCells, address_from_state_init, build_external_message, build_internal_message,
    build_w5r1_state_init, make_w5r1_wallet_id, pack_w5_signed_body, parse_w5_init_data,
    serialize_out_list, serialize_send_msg_action, unsigned_w5_body,
};
use crate::exact::types;
use crate::exact::{ExactTvmPayload, TvmExact};
use crate::trace::{
    normalize_address_or_null, parse_trace_transactions, trace_transaction_compute_fees,
    trace_transaction_fwd_fees, trace_transaction_storage_fees, transaction_succeeded,
};
use crate::{
    DEFAULT_JETTON_WALLET_MESSAGE_AMOUNT, DEFAULT_TONCENTER_EMULATION_TIMEOUT_SECONDS,
    DEFAULT_TVM_EMULATION_ADDRESS, DEFAULT_TVM_EMULATION_RELAY_AMOUNT, DEFAULT_TVM_EMULATION_SEQNO,
    DEFAULT_TVM_EMULATION_WALLET_ID, DEFAULT_TVM_INNER_GAS_BUFFER, DEFAULT_VALID_UNTIL_OFFSET,
    DEFAULT_W5R1_SUBWALLET_NUMBER, EXTERNAL_SIGNED_OP, INTERNAL_SIGNED_OP, SEND_MODE_IGNORE_ERRORS,
    SEND_MODE_PAY_FEES_SEPARATELY,
};

/// Local W5R1 signer wrapping a 32-byte Ed25519 seed.
#[derive(Clone)]
pub struct TvmW5Signer {
    signing_key: SigningKey,
    address: TvmAddress,
    network: TvmChainReference,
    wallet_id: u32,
    state_init: StateInitCells,
}

impl std::fmt::Debug for TvmW5Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TvmW5Signer")
            .field("address", &self.address)
            .field("network", &self.network)
            .field("wallet_id", &self.wallet_id)
            .finish_non_exhaustive()
    }
}

impl TvmW5Signer {
    /// Constructs a W5R1 signer from a 32-byte seed.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Signing`] if the seed is not 32 bytes or wallet
    /// construction fails.
    pub fn from_seed(seed: &[u8], network: TvmChainReference) -> Result<Self, ClientError> {
        Self::from_seed_with_params(seed, network, 0, DEFAULT_W5R1_SUBWALLET_NUMBER)
    }

    /// Constructs a W5R1 signer with an explicit workchain / subwallet.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Signing`] if the seed is not 32 bytes or wallet
    /// construction fails.
    pub fn from_seed_with_params(
        seed: &[u8],
        network: TvmChainReference,
        workchain: i8,
        subwallet_number: u16,
    ) -> Result<Self, ClientError> {
        if seed.len() != 32 {
            return Err(ClientError::Signing(
                "TVM client seed must be 32 bytes".to_owned(),
            ));
        }
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(seed);
        let signing_key = SigningKey::from_bytes(&bytes);
        let wallet_id = make_w5r1_wallet_id(network, workchain, subwallet_number)
            .map_err(|e| ClientError::Signing(e.to_string()))?;
        let public_key = signing_key.verifying_key().to_bytes();
        let state_init = build_w5r1_state_init(&public_key, wallet_id)
            .map_err(|e| ClientError::Signing(e.to_string()))?;
        let address = address_from_state_init(&state_init, i32::from(workchain))
            .map_err(|e| ClientError::Signing(e.to_string()))?;
        Ok(Self {
            signing_key,
            address,
            network,
            wallet_id,
            state_init,
        })
    }

    /// Raw payer address.
    #[must_use]
    pub const fn address(&self) -> &TvmAddress {
        &self.address
    }

    /// Network this signer is bound to.
    #[must_use]
    pub const fn network(&self) -> TvmChainReference {
        self.network
    }

    /// W5 `walletId`.
    #[must_use]
    pub const fn wallet_id(&self) -> u32 {
        self.wallet_id
    }

    /// Signs `message` (typically a cell hash).
    #[must_use]
    pub fn sign_message(&self, message: &[u8]) -> [u8; 64] {
        self.signing_key.sign(message).to_bytes()
    }

    /// Builds a base64 settlement BoC for one jetton-wallet out message.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if cells cannot be built.
    pub fn sign_transfer(
        &self,
        seqno: u32,
        valid_until: u32,
        dest: &TvmAddress,
        amount: u128,
        body: &Cell,
        include_state_init: bool,
    ) -> Result<String, ClientError> {
        let dest_ton = dest
            .to_ton()
            .map_err(|e| ClientError::Signing(e.to_string()))?;
        let out_message = build_internal_message(&dest_ton, amount, true, None, body)
            .map_err(|e| ClientError::Signing(e.to_string()))?;
        let action = serialize_send_msg_action(&out_message, SEND_MODE_PAY_FEES_SEPARATELY)
            .map_err(|e| ClientError::Signing(e.to_string()))?;
        let actions =
            serialize_out_list(&[action]).map_err(|e| ClientError::Signing(e.to_string()))?;
        let unsigned = unsigned_w5_body(
            INTERNAL_SIGNED_OP,
            self.wallet_id,
            valid_until,
            seqno,
            Some(&actions),
        )
        .map_err(|e| ClientError::Signing(e.to_string()))?;
        let mut hash = [0u8; 32];
        hash.copy_from_slice(unsigned.cell_hash().as_slice());
        let signature = self.sign_message(&hash);
        let transfer_body = pack_w5_signed_body(
            INTERNAL_SIGNED_OP,
            self.wallet_id,
            valid_until,
            seqno,
            &actions,
            &signature,
        )
        .map_err(|e| ClientError::Signing(e.to_string()))?;
        let payer = self
            .address
            .to_ton()
            .map_err(|e| ClientError::Signing(e.to_string()))?;
        let settlement = build_internal_message(
            &payer,
            0,
            true,
            include_state_init.then_some(&self.state_init),
            &transfer_body,
        )
        .map_err(|e| ClientError::Signing(e.to_string()))?;
        encode_base64_boc(&settlement).map_err(|e| ClientError::Signing(e.to_string()))
    }
}

/// TON exact scheme client for building and signing payment payloads.
#[derive(Clone)]
pub struct TvmExactClient<S, R> {
    signer: S,
    rpc: R,
}

impl<S, R> std::fmt::Debug for TvmExactClient<S, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TvmExactClient").finish_non_exhaustive()
    }
}

impl<S, R> TvmExactClient<S, R> {
    /// Creates a new TON exact client.
    pub const fn new(signer: S, rpc: R) -> Self {
        Self { signer, rpc }
    }
}

impl<S, R> SchemeId for TvmExactClient<S, R> {
    fn namespace(&self) -> &str {
        TvmExact.namespace()
    }

    fn scheme(&self) -> &str {
        TvmExact.scheme()
    }
}

impl<S, R> r402_core::scheme::Sealed for TvmExactClient<S, R> {}

impl<S, R> SchemeClient for TvmExactClient<S, R>
where
    S: AsRef<TvmW5Signer> + Send + Sync + Clone + 'static,
    R: TvmRpc + Clone + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: types::v2::PaymentRequirements = v.as_concrete()?;
                let chain_id = requirements.network.clone();
                if chain_id.namespace() != "tvm" {
                    return None;
                }
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
                        requirements,
                        resource: payment_required.resource.clone(),
                    }),
                })
            })
            .collect()
    }

    fn find_default_asset(&self, asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
        crate::find_default_tvm_asset(asset, network)
    }
}

impl AsRef<Self> for TvmW5Signer {
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
    S: AsRef<TvmW5Signer> + Send + Sync,
    R: TvmRpc,
{
    fn sign_payment(&self) -> r402_core::facilitator::BoxFuture<'_, Result<String, ClientError>> {
        Box::pin(async move {
            let boc =
                create_settlement_boc(self.signer.as_ref(), &self.rpc, &self.requirements).await?;
            let payload = types::v2::PaymentPayload::new(
                self.requirements.clone(),
                ExactTvmPayload {
                    settlement_boc: boc,
                    asset: self.requirements.asset.clone(),
                },
            )
            .with_resource(self.resource.clone());
            let json = serde_json::to_vec(&payload)?;
            let encoded = Base64Bytes::encode(&json);
            Ok(encoded.to_string())
        })
    }
}

/// Builds a base64 settlement BoC for `requirements`.
///
/// # Errors
///
/// Returns [`ClientError`] if REST reads fail, fees are not sponsored, or
/// signing fails.
pub async fn create_settlement_boc<R: TvmRpc>(
    signer: &TvmW5Signer,
    rpc: &R,
    requirements: &types::v2::PaymentRequirements,
) -> Result<String, ClientError> {
    let chain = TvmChainReference::try_from(requirements.network.clone())
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    if signer.network() != chain {
        return Err(ClientError::Signing(format!(
            "Signer network {} does not match requirements network {chain}",
            signer.network()
        )));
    }
    let extra = requirements.extra.clone().unwrap_or_default();
    if !extra.are_fees_sponsored {
        return Err(ClientError::Signing(
            "Exact TVM scheme requires extra.areFeesSponsored to be true".to_owned(),
        ));
    }
    let asset = &requirements.asset;
    let payer = signer.address();
    let source_wallet = rpc
        .get_jetton_wallet(asset.as_str(), payer.as_str())
        .await
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let account = rpc
        .get_account_state(payer.as_str())
        .await
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let include_state_init = !account.is_active;
    let seqno = if account.is_uninitialized {
        0
    } else {
        let init = account.state_init.as_ref().ok_or_else(|| {
            ClientError::Signing("active W5 account missing stateInit".to_owned())
        })?;
        parse_w5_init_data(init)
            .map_err(|e| ClientError::Signing(e.to_string()))?
            .seqno
    };
    let timeout_seconds = if requirements.max_timeout_seconds == 0 {
        DEFAULT_VALID_UNTIL_OFFSET
    } else {
        requirements.max_timeout_seconds
    };
    let valid_until = now_unix().saturating_add(if timeout_seconds > 10 {
        timeout_seconds.saturating_sub(5)
    } else {
        timeout_seconds.div_ceil(2)
    });
    let valid_until = u32::try_from(valid_until)
        .map_err(|_| ClientError::Signing("validUntil overflow".to_owned()))?;
    let amount = requirements
        .amount
        .as_u128()
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let transfer_body = build_jetton_transfer_body(amount, &requirements.pay_to, &extra)
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let required_inner = estimate_required_inner_value(
        signer,
        rpc,
        &source_wallet,
        extra.forward_ton_amount_u128().unwrap_or(0),
        seqno,
        valid_until,
        &transfer_body,
        include_state_init,
    )
    .await?;
    signer.sign_transfer(
        seqno,
        valid_until,
        &source_wallet,
        required_inner,
        &transfer_body,
        include_state_init,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "mirrors the TS estimateRequiredInnerValue argument list"
)]
async fn estimate_required_inner_value<R: TvmRpc>(
    signer: &TvmW5Signer,
    rpc: &R,
    source_wallet: &TvmAddress,
    forward_ton_amount: u128,
    seqno: u32,
    valid_until: u32,
    transfer_body: &Cell,
    include_state_init: bool,
) -> Result<u128, ClientError> {
    let provisional = DEFAULT_JETTON_WALLET_MESSAGE_AMOUNT.saturating_add(forward_ton_amount);
    let source_ton = source_wallet
        .to_ton()
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let payer_out = build_internal_message(&source_ton, provisional, true, None, transfer_body)
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let action = serialize_send_msg_action(&payer_out, SEND_MODE_PAY_FEES_SEPARATELY)
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let actions = serialize_out_list(&[action]).map_err(|e| ClientError::Signing(e.to_string()))?;
    let unsigned = unsigned_w5_body(
        INTERNAL_SIGNED_OP,
        signer.wallet_id,
        valid_until,
        seqno,
        Some(&actions),
    )
    .map_err(|e| ClientError::Signing(e.to_string()))?;
    let mut hash = [0u8; 32];
    hash.copy_from_slice(unsigned.cell_hash().as_slice());
    let payer_body = pack_w5_signed_body(
        INTERNAL_SIGNED_OP,
        signer.wallet_id,
        valid_until,
        seqno,
        &actions,
        &signer.sign_message(&hash),
    )
    .map_err(|e| ClientError::Signing(e.to_string()))?;
    let payer = signer
        .address
        .to_ton()
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let relay = build_internal_message(
        &payer,
        DEFAULT_TVM_EMULATION_RELAY_AMOUNT,
        true,
        include_state_init.then_some(&signer.state_init),
        &payer_body,
    )
    .map_err(|e| ClientError::Signing(e.to_string()))?;
    let relay_action = serialize_send_msg_action(
        &relay,
        SEND_MODE_PAY_FEES_SEPARATELY + SEND_MODE_IGNORE_ERRORS,
    )
    .map_err(|e| ClientError::Signing(e.to_string()))?;
    let relay_actions =
        serialize_out_list(&[relay_action]).map_err(|e| ClientError::Signing(e.to_string()))?;
    let dummy_unsigned = unsigned_w5_body(
        EXTERNAL_SIGNED_OP,
        DEFAULT_TVM_EMULATION_WALLET_ID,
        valid_until,
        DEFAULT_TVM_EMULATION_SEQNO,
        Some(&relay_actions),
    )
    .map_err(|e| ClientError::Signing(e.to_string()))?;
    let mut dummy_hash = [0u8; 32];
    dummy_hash.copy_from_slice(dummy_unsigned.cell_hash().as_slice());
    let dummy_body = pack_w5_signed_body(
        EXTERNAL_SIGNED_OP,
        DEFAULT_TVM_EMULATION_WALLET_ID,
        valid_until,
        DEFAULT_TVM_EMULATION_SEQNO,
        &relay_actions,
        &signer.sign_message(&dummy_hash),
    )
    .map_err(|e| ClientError::Signing(e.to_string()))?;
    let dummy_dest: TonAddress = DEFAULT_TVM_EMULATION_ADDRESS
        .parse()
        .map_err(|e: tonlib_core::TonAddressParseError| ClientError::Signing(e.to_string()))?;
    let external = build_external_message(&dummy_dest, None, &dummy_body)
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let boc = tonlib_core::cell::BagOfCells::from_root(external)
        .serialize(true)
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let trace = rpc
        .emulate_trace(&boc, true, DEFAULT_TONCENTER_EMULATION_TIMEOUT_SECONDS)
        .await
        .map_err(|e| ClientError::Signing(e.to_string()))?;
    let transactions =
        parse_trace_transactions(&trace).map_err(|e| ClientError::Signing(e.to_string()))?;
    let source_tx = find_source_wallet_transaction(&transactions, source_wallet, signer.address())?;
    let receiver_tx = find_receiver_wallet_transaction(&transactions, source_wallet)?;
    Ok(DEFAULT_TVM_INNER_GAS_BUFFER
        .saturating_add(trace_transaction_fwd_fees(
            source_tx,
            if forward_ton_amount > 0 { 2 } else { 1 },
        ))
        .saturating_add(trace_transaction_compute_fees(source_tx))
        .saturating_add(trace_transaction_compute_fees(receiver_tx))
        .saturating_add(forward_ton_amount)
        .saturating_add(trace_transaction_storage_fees(source_tx)))
}

fn find_source_wallet_transaction<'a>(
    transactions: &[&'a serde_json::Value],
    source_wallet: &TvmAddress,
    payer: &TvmAddress,
) -> Result<&'a serde_json::Value, ClientError> {
    for transaction in transactions {
        if normalize_address_or_null(transaction.get("account")).as_ref() != Some(source_wallet) {
            continue;
        }
        if !transaction_succeeded(transaction) {
            continue;
        }
        let in_msg = transaction.get("in_msg");
        if in_msg
            .and_then(|m| m.get("decoded_opcode"))
            .and_then(serde_json::Value::as_str)
            != Some("jetton_transfer")
        {
            continue;
        }
        if normalize_address_or_null(in_msg.and_then(|m| m.get("source"))).as_ref() != Some(payer) {
            continue;
        }
        return Ok(*transaction);
    }
    Err(ClientError::Signing(
        "Trace does not contain the expected source jetton wallet transaction".to_owned(),
    ))
}

fn find_receiver_wallet_transaction<'a>(
    transactions: &[&'a serde_json::Value],
    source_wallet: &TvmAddress,
) -> Result<&'a serde_json::Value, ClientError> {
    for transaction in transactions {
        if !transaction_succeeded(transaction) {
            continue;
        }
        let in_msg = transaction.get("in_msg");
        if in_msg
            .and_then(|m| m.get("decoded_opcode"))
            .and_then(serde_json::Value::as_str)
            != Some("jetton_internal_transfer")
        {
            continue;
        }
        if normalize_address_or_null(in_msg.and_then(|m| m.get("source"))).as_ref()
            != Some(source_wallet)
        {
            continue;
        }
        return Ok(*transaction);
    }
    Err(ClientError::Signing(
        "Trace does not contain the expected destination jetton wallet transaction".to_owned(),
    ))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl From<BocError> for ClientError {
    fn from(value: BocError) -> Self {
        Self::Signing(value.to_string())
    }
}
