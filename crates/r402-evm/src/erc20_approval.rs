//! ERC-20 approval gas-sponsoring extension wire types (exact + upto).
//!
//! Official key: `erc20ApprovalGasSponsoring`. Clients attach a signed
//! `approve(Permit2, max)` transaction; facilitators fund native gas if needed,
//! broadcast the envelope, then Permit2 `settle`.

use alloy_primitives::Address;
use r402_protocol::payment::{ExtensionEntry, Extensions};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable extension identifier on the wire.
pub const ERC20_APPROVAL_GAS_SPONSORING_KEY: &str = "erc20ApprovalGasSponsoring";

/// Schema version written into client-populated `info.version`.
pub const ERC20_APPROVAL_GAS_SPONSORING_VERSION: &str = "1";

/// Official gas limit for the sponsored `approve` transaction.
pub const ERC20_APPROVE_GAS_LIMIT: u64 = 70_000;

/// Fallback `maxFeePerGas` (1 gwei) when fee estimation is unavailable.
pub const DEFAULT_MAX_FEE_PER_GAS: u128 = 1_000_000_000;

/// Fallback `maxPriorityFeePerGas` (0.1 gwei) when fee estimation is unavailable.
pub const DEFAULT_MAX_PRIORITY_FEE_PER_GAS: u128 = 100_000_000;

/// Buyer-signed ERC-20 `approve` transaction for gasless Permit2 allowance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Erc20ApprovalGasSponsoringInfo {
    /// Token owner (== payment payer).
    pub from: Address,
    /// ERC-20 token contract.
    pub asset: Address,
    /// Spender — MUST be the canonical Permit2 address.
    pub spender: Address,
    /// Approved allowance (uint256 decimal string). Official always uses `maxUint256`.
    pub amount: String,
    /// RLP-encoded signed EIP-1559 `approve` transaction (`0x`-prefixed hex).
    pub signed_transaction: String,
    /// Schema version (`"1"`).
    pub version: String,
}

/// Errors raised while extracting [`Erc20ApprovalGasSponsoringInfo`] from extensions.
#[derive(Debug, Error)]
pub enum Erc20ApprovalParseError {
    /// Extension JSON did not match the expected shape.
    #[error("invalid erc20ApprovalGasSponsoring extension payload: {0}")]
    Invalid(#[from] serde_json::Error),
}

impl Erc20ApprovalGasSponsoringInfo {
    /// Decodes the extension from a payload extensions map.
    ///
    /// # Errors
    ///
    /// Malformed JSON.
    pub fn from_extensions(
        extensions: &Extensions,
    ) -> Result<Option<Self>, Erc20ApprovalParseError> {
        let Some(entry) = extensions.get(ERC20_APPROVAL_GAS_SPONSORING_KEY) else {
            return Ok(None);
        };
        let parsed: Self = match entry {
            ExtensionEntry::Structured { info, .. } => serde_json::from_value(info.clone())?,
            ExtensionEntry::Raw(value) => serde_json::from_value(value.clone())?,
        };
        Ok(Some(parsed))
    }

    /// Builds a structured extension entry for payload insertion.
    ///
    /// # Errors
    ///
    /// Serde failures (should not occur for this type).
    pub fn to_extension_entry(&self) -> Result<ExtensionEntry, serde_json::Error> {
        Ok(ExtensionEntry::info(serde_json::to_value(self)?))
    }
}

/// Maximum native-token deficit the facilitator will transfer to a payer.
pub const MAX_ERC20_SPONSOR_WEI: u128 = 2_000_000_000_000_000;

const APPROVE_SELECTOR: [u8; 4] = [0x09, 0x5e, 0xa7, 0xb3];

/// Decoded buyer-signed `approve` envelope ready to simulate or broadcast.
#[cfg(feature = "facilitator")]
#[derive(Debug, Clone)]
pub struct ValidatedErc20Approval {
    /// Original extension payload.
    pub info: Erc20ApprovalGasSponsoringInfo,
    /// Recovered EIP-1559 envelope.
    pub envelope: alloy_consensus::TxEnvelope,
    /// EIP-2718 bytes for `eth_sendRawTransaction`.
    pub encoded: Vec<u8>,
    /// Envelope gas limit.
    pub gas_limit: u64,
    /// Envelope `maxFeePerGas`.
    pub max_fee_per_gas: u128,
    /// Envelope `maxPriorityFeePerGas`.
    pub max_priority_fee_per_gas: u128,
    /// `gas_limit * max_fee_per_gas` (not the funding cap).
    pub native_cost: alloy_primitives::U256,
}

/// Structural signed-tx checks (no RPC nonce / base-fee).
#[cfg(feature = "facilitator")]
pub(crate) fn decode_erc20_approval_structural(
    info: &Erc20ApprovalGasSponsoringInfo,
    payer: Address,
    token: Address,
    chain_id: u64,
) -> Result<ValidatedErc20Approval, r402_protocol::error::VerificationError> {
    decode_erc20_approval_envelope(info, payer, token, chain_id)
}

/// Full RPC-field validation. Has no balance argument and does not cap funding.
///
/// # Errors
///
/// Wire `erc20_approval_*` codes when the envelope cannot be broadcast.
#[cfg(feature = "facilitator")]
pub fn validate_erc20_approval_for_payment(
    info: &Erc20ApprovalGasSponsoringInfo,
    payer: Address,
    token: Address,
    chain_id: u64,
    on_chain_nonce: u64,
    current_base_fee: u128,
) -> Result<ValidatedErc20Approval, r402_protocol::error::VerificationError> {
    use alloy_consensus::transaction::Transaction;
    use r402_protocol::error::VerificationError;

    let decoded = decode_erc20_approval_envelope(info, payer, token, chain_id)?;
    if decoded.envelope.nonce() != on_chain_nonce {
        return Err(VerificationError::from_wire(
            "erc20_approval_nonce_mismatch",
        ));
    }
    if decoded.max_priority_fee_per_gas == 0
        || decoded.max_fee_per_gas < current_base_fee
        || decoded.max_fee_per_gas < decoded.max_priority_fee_per_gas
    {
        return Err(VerificationError::from_wire("erc20_approval_fee_too_low"));
    }
    Ok(decoded)
}

#[cfg(feature = "facilitator")]
#[allow(
    clippy::too_many_lines,
    reason = "each wire code is a distinct closed check"
)]
fn decode_erc20_approval_envelope(
    info: &Erc20ApprovalGasSponsoringInfo,
    payer: Address,
    token: Address,
    chain_id: u64,
) -> Result<ValidatedErc20Approval, r402_protocol::error::VerificationError> {
    use alloy_consensus::TxEnvelope;
    use alloy_consensus::transaction::{SignerRecoverable, Transaction};
    use alloy_eips::eip2718::Decodable2718;
    use alloy_primitives::{Address, U256, hex};
    use r402_protocol::error::VerificationError;

    use crate::permit2::PERMIT2_ADDRESS;

    if info.from != payer {
        return Err(VerificationError::from_wire("erc20_approval_from_mismatch"));
    }
    if info.asset != token {
        return Err(VerificationError::from_wire(
            "erc20_approval_asset_mismatch",
        ));
    }
    if info.spender != PERMIT2_ADDRESS {
        return Err(VerificationError::from_wire(
            "erc20_approval_spender_not_permit2",
        ));
    }
    if info.version.is_empty() {
        return Err(VerificationError::from_wire(
            "invalid_erc20_approval_extension_format",
        ));
    }

    let hex_body = info
        .signed_transaction
        .strip_prefix("0x")
        .or_else(|| info.signed_transaction.strip_prefix("0X"))
        .unwrap_or(info.signed_transaction.as_str());
    let encoded = hex::decode(hex_body)
        .map_err(|_| VerificationError::from_wire("erc20_approval_tx_parse_failed"))?;
    let envelope = TxEnvelope::decode_2718(&mut encoded.as_slice())
        .map_err(|_| VerificationError::from_wire("erc20_approval_tx_parse_failed"))?;
    let TxEnvelope::Eip1559(_) = &envelope else {
        return Err(VerificationError::from_wire("erc20_approval_not_eip1559"));
    };
    if envelope.chain_id() != Some(chain_id) {
        return Err(VerificationError::from_wire(
            "erc20_approval_chain_id_mismatch",
        ));
    }
    if !envelope.value().is_zero() {
        return Err(VerificationError::from_wire("erc20_approval_value_nonzero"));
    }
    let Some(to) = envelope.to() else {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_wrong_target",
        ));
    };
    if to != token {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_wrong_target",
        ));
    }
    let data = envelope.input();
    let Some(selector) = data.get(..4) else {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_wrong_selector",
        ));
    };
    if selector != APPROVE_SELECTOR {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_wrong_selector",
        ));
    }
    let Some(spender_word) = data.get(4..36) else {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_invalid_calldata",
        ));
    };
    let Some(spender_bytes) = spender_word.get(12..) else {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_invalid_calldata",
        ));
    };
    let Ok(spender) = Address::try_from(spender_bytes) else {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_invalid_calldata",
        ));
    };
    if spender != PERMIT2_ADDRESS {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_wrong_spender",
        ));
    }
    let Some(amount_word) = data.get(36..68) else {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_invalid_calldata",
        ));
    };
    let calldata_amount = U256::from_be_slice(amount_word);
    let expected_amount = info
        .amount
        .parse::<U256>()
        .map_err(|_| VerificationError::from_wire("erc20_approval_amount_mismatch"))?;
    if calldata_amount != expected_amount {
        return Err(VerificationError::from_wire(
            "erc20_approval_amount_mismatch",
        ));
    }
    let recovered = envelope
        .recover_signer()
        .map_err(|_| VerificationError::from_wire("erc20_approval_tx_invalid_signature"))?;
    if recovered != payer {
        return Err(VerificationError::from_wire(
            "erc20_approval_tx_signer_mismatch",
        ));
    }

    let gas_limit = envelope.gas_limit();
    let max_fee_per_gas = envelope.max_fee_per_gas();
    let max_priority_fee_per_gas = envelope.max_priority_fee_per_gas().unwrap_or(0);
    let native_cost = U256::from(gas_limit) * U256::from(max_fee_per_gas);
    Ok(ValidatedErc20Approval {
        info: info.clone(),
        envelope,
        encoded,
        gas_limit,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        native_cost,
    })
}

/// Rejects a funding deficit above [`MAX_ERC20_SPONSOR_WEI`].
#[cfg(feature = "facilitator")]
pub(crate) fn funding_deficit(
    native_cost: alloy_primitives::U256,
    balance: alloy_primitives::U256,
) -> Result<alloy_primitives::U256, r402_protocol::error::VerificationError> {
    use alloy_primitives::U256;
    use r402_protocol::error::VerificationError;

    let deficit = native_cost.saturating_sub(balance);
    if deficit > U256::from(MAX_ERC20_SPONSOR_WEI) {
        return Err(VerificationError::from_wire(
            "erc20_approval_funding_exceeds_cap",
        ));
    }
    Ok(deficit)
}

/// Fail closed when the pinned funder cannot cover `fund_value`.
#[cfg(feature = "facilitator")]
pub(crate) async fn assert_facilitator_can_fund<P: alloy_provider::Provider>(
    provider: &P,
    fund_from: Address,
    fund_value: alloy_primitives::U256,
) -> Result<(), crate::error::Eip155ExactError> {
    use r402_protocol::error::VerificationError;

    if fund_value.is_zero() {
        return Ok(());
    }
    let hot_balance = provider.get_balance(fund_from).await?;
    if hot_balance < fund_value {
        return Err(VerificationError::from_wire("erc20_approval_facilitator_underfunded").into());
    }
    Ok(())
}

/// First configured hot-wallet address.
#[cfg(feature = "facilitator")]
pub(crate) fn first_hot_wallet(
    provider: &impl r402_protocol::network::ChainProvider,
) -> Result<Address, r402_protocol::error::VerificationError> {
    use std::str::FromStr;

    use alloy_primitives::Address;
    use r402_protocol::error::VerificationError;

    let raw = provider
        .signer_addresses()
        .into_iter()
        .next()
        .ok_or_else(|| VerificationError::from_wire("erc20_approval_facilitator_underfunded"))?;
    Address::from_str(&raw)
        .map_err(|_| VerificationError::from_wire("erc20_approval_facilitator_underfunded"))
}

/// Re-check nonce/fees/deficit with live chain state.
#[cfg(feature = "facilitator")]
pub(crate) async fn rpc_validate_covering_approval<P: alloy_provider::Provider>(
    provider: &P,
    approval: &ValidatedErc20Approval,
    payer: Address,
    token: Address,
    chain_id: u64,
) -> Result<ValidatedErc20Approval, crate::error::Eip155ExactError> {
    let on_chain_nonce = provider.get_transaction_count(payer).await?;
    let base_fee = current_base_fee(provider).await?;
    let validated = validate_erc20_approval_for_payment(
        &approval.info,
        payer,
        token,
        chain_id,
        on_chain_nonce,
        base_fee,
    )?;
    let native_balance = provider.get_balance(payer).await?;
    funding_deficit(validated.native_cost, native_balance)?;
    Ok(validated)
}

/// Latest header `baseFeePerGas`.
#[cfg(feature = "facilitator")]
pub(crate) async fn current_base_fee<P: alloy_provider::Provider>(
    provider: &P,
) -> Result<u128, crate::error::Eip155ExactError> {
    use alloy_consensus::BlockHeader;
    use alloy_network::BlockResponse;
    use alloy_rpc_types_eth::BlockNumberOrTag;
    use r402_protocol::error::VerificationError;

    let block = provider
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await
        .map_err(crate::error::Eip155ExactError::from)?
        .ok_or_else(|| VerificationError::from_wire("erc20_approval_missing_base_fee"))?;
    block
        .header()
        .base_fee_per_gas()
        .map(u128::from)
        .ok_or_else(|| VerificationError::from_wire("erc20_approval_missing_base_fee").into())
}

#[cfg(feature = "facilitator")]
fn is_rpc_method_missing(err: &alloy_transport::TransportError) -> bool {
    fn message_indicates_missing(message: &str) -> bool {
        let lower = message.to_ascii_lowercase();
        lower.contains("eth_simulatev1")
            || lower.contains("method not found")
            || lower.contains("method does not exist")
    }
    match err {
        alloy_transport::RpcError::ErrorResp(payload) => {
            payload.code == -32601 || message_indicates_missing(payload.message.as_ref())
        }
        other => message_indicates_missing(&other.to_string()),
    }
}

/// `eth_simulateV1` of fund (optional) + unsigned approve + settle.
///
/// `validation` stays false so the unsigned `from=payer` approve is accepted.
#[cfg(feature = "facilitator")]
#[allow(
    clippy::too_many_arguments,
    reason = "simulate payload needs every call site's addresses and gas"
)]
pub(crate) async fn simulate_permit2_settle_with_erc20_approval<P: alloy_provider::Provider>(
    provider: &P,
    approval: &ValidatedErc20Approval,
    payer: Address,
    fund_from: Address,
    settle_from: Address,
    token: Address,
    fund_value: alloy_primitives::U256,
    settle_to: Address,
    settle_calldata: alloy_primitives::Bytes,
) -> Result<(), crate::error::Eip155ExactError> {
    use alloy_consensus::transaction::Transaction;
    use alloy_network::TransactionBuilder;
    use alloy_rpc_types_eth::TransactionRequest;
    use alloy_rpc_types_eth::simulate::{SimBlock, SimulatePayload};
    use r402_protocol::error::VerificationError;

    use crate::error::Eip155ExactError;

    let mut block = SimBlock::default();
    if !fund_value.is_zero() {
        block = block.call(
            TransactionRequest::default()
                .with_from(fund_from)
                .with_to(payer)
                .with_value(fund_value),
        );
    }
    block = block.call(
        TransactionRequest::default()
            .with_from(payer)
            .with_to(token)
            .with_input(approval.envelope.input().clone())
            .with_gas_limit(approval.gas_limit)
            .with_max_fee_per_gas(approval.max_fee_per_gas)
            .with_max_priority_fee_per_gas(approval.max_priority_fee_per_gas),
    );
    block = block.call(
        TransactionRequest::default()
            .with_from(settle_from)
            .with_to(settle_to)
            .with_input(settle_calldata),
    );
    let expected_call_count = block.calls.len();
    let payload = SimulatePayload {
        block_state_calls: vec![block],
        trace_transfers: false,
        validation: false,
        return_full_transactions: false,
    };

    let sim_fut = provider.simulate(&payload);
    let blocks = {
        #[cfg(feature = "telemetry")]
        {
            use tracing::Instrument;
            sim_fut
                .instrument(tracing::info_span!(
                    "simulate_permit2_settle_with_erc20",
                    from = %payer,
                    token = %token,
                    fund_value = %fund_value,
                    otel.kind = "client",
                ))
                .await
        }
        #[cfg(not(feature = "telemetry"))]
        {
            sim_fut.await
        }
    };
    let blocks = match blocks {
        Ok(blocks) => blocks,
        Err(err) if is_rpc_method_missing(&err) => {
            return Err(Eip155ExactError::RpcMethodMissing);
        }
        Err(err) => return Err(Eip155ExactError::Transport(err)),
    };
    let Some(sim_block) = blocks.first() else {
        return Err(VerificationError::SimulationFailed("empty eth_simulateV1".into()).into());
    };
    if sim_block.calls.len() != expected_call_count {
        return Err(VerificationError::SimulationFailed(format!(
            "eth_simulateV1 expected {expected_call_count} calls, got {}",
            sim_block.calls.len()
        ))
        .into());
    }
    for (i, call) in sim_block.calls.iter().enumerate() {
        if !call.status {
            return Err(VerificationError::SimulationFailed(format!(
                "eth_simulateV1 call {i} status=false error={:?}",
                call.error
            ))
            .into());
        }
    }
    Ok(())
}

#[cfg(feature = "facilitator")]
fn erc20_approval_lock(
    chain_id: u64,
    payer: Address,
    token: Address,
) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    use std::sync::{Arc, OnceLock};

    use dashmap::DashMap;
    use tokio::sync::Mutex;

    type ApprovalLocks = DashMap<(u64, Address, Address), Arc<Mutex<()>>>;
    static LOCKS: OnceLock<ApprovalLocks> = OnceLock::new();
    let map = LOCKS.get_or_init(DashMap::new);
    let entry = map
        .entry((chain_id, payer, token))
        .or_insert_with(|| Arc::new(Mutex::new(())));
    Arc::clone(entry.value())
}

/// Broadcasts native funding (if needed) then the buyer-signed `approve`.
///
/// Allowance is checked first so a retry after a landed approve skips the envelope.
#[cfg(feature = "facilitator")]
pub(crate) async fn relay_erc20_approval<P, E>(
    provider: &P,
    approval: &ValidatedErc20Approval,
    payer: Address,
    required: alloy_primitives::U256,
    fund_from: Address,
) -> Result<Option<alloy_primitives::TxHash>, crate::error::Eip155ExactError>
where
    P: crate::chain::Eip155MetaTransactionProvider<Error = E> + Sync,
    crate::error::Eip155ExactError: From<E>,
{
    use alloy_primitives::Bytes;
    use alloy_provider::Provider;

    use crate::chain::contracts::IERC20;
    use crate::chain::{Eip155MetaTransactionProvider, MetaTransaction};
    use crate::error::Eip155ExactError;
    use crate::permit2::PERMIT2_ADDRESS;

    let chain_id = provider.chain().inner();
    let token = approval.info.asset;
    let lock = erc20_approval_lock(chain_id, payer, token);
    let _guard = lock.lock().await;

    let erc20 = IERC20::new(token, provider.inner());
    let allowance = erc20.allowance(payer, PERMIT2_ADDRESS).call().await?;
    if allowance >= required {
        return Ok(None);
    }

    let on_chain_nonce = provider.inner().get_transaction_count(payer).await?;
    let current_base_fee = current_base_fee(provider.inner()).await?;
    let _ = validate_erc20_approval_for_payment(
        &approval.info,
        payer,
        token,
        chain_id,
        on_chain_nonce,
        current_base_fee,
    )?;

    let balance = provider.inner().get_balance(payer).await?;
    let deficit = funding_deficit(approval.native_cost, balance)?;
    if !deficit.is_zero() {
        let tx_fut = Eip155MetaTransactionProvider::send_transaction(
            provider,
            MetaTransaction {
                to: payer,
                calldata: Bytes::new(),
                confirmations: 1,
                from: Some(fund_from),
                value: deficit,
            },
        );
        let receipt = {
            #[cfg(feature = "telemetry")]
            {
                use tracing::Instrument;
                tx_fut
                    .instrument(tracing::info_span!(
                        "erc20_approval_fund",
                        from = %fund_from,
                        value = %deficit,
                        otel.kind = "client",
                    ))
                    .await
            }
            #[cfg(not(feature = "telemetry"))]
            {
                tx_fut.await
            }
        };
        match receipt {
            Ok(receipt) if receipt.status() => {
                #[cfg(feature = "telemetry")]
                tracing::info!(tx = %receipt.transaction_hash, value = %deficit, "erc20_approval_fund");
            }
            Ok(_) | Err(_) => return Err(Eip155ExactError::Erc20ApprovalFundingFailed),
        }
    }

    let raw_fut =
        Eip155MetaTransactionProvider::send_raw_transaction(provider, &approval.encoded, 1);
    let receipt = {
        #[cfg(feature = "telemetry")]
        {
            use tracing::Instrument;
            raw_fut
                .instrument(tracing::info_span!(
                    "erc20_approval_send_raw",
                    from = %payer,
                    token = %token,
                    otel.kind = "client",
                ))
                .await
        }
        #[cfg(not(feature = "telemetry"))]
        {
            raw_fut.await
        }
    };
    let receipt = match receipt {
        Ok(receipt) => receipt,
        Err(e) => {
            return Err(match Eip155ExactError::from(e) {
                Eip155ExactError::ReceiptWait { hash, .. } => {
                    Eip155ExactError::Erc20ApprovalTxFailed(hash)
                }
                other => other,
            });
        }
    };
    if !receipt.status() {
        return Err(Eip155ExactError::Erc20ApprovalTxFailed(
            receipt.transaction_hash,
        ));
    }
    Ok(Some(receipt.transaction_hash))
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;
    use r402_protocol::payment::Extensions;

    use super::*;

    fn sample() -> Erc20ApprovalGasSponsoringInfo {
        Erc20ApprovalGasSponsoringInfo {
            from: Address::repeat_byte(0xAA),
            asset: Address::repeat_byte(0xBB),
            spender: Address::repeat_byte(0xCC),
            amount:
                "115792089237316195423570985008687907853269984665640564039457584007913129639935"
                    .into(),
            signed_transaction: "0xdeadbeef".into(),
            version: ERC20_APPROVAL_GAS_SPONSORING_VERSION.into(),
        }
    }

    #[test]
    fn wire_uses_signed_transaction_camel_case() {
        let v = serde_json::to_value(sample()).unwrap();
        assert!(v.get("signedTransaction").is_some());
        assert!(v.get("signed_transaction").is_none());
        assert_eq!(v.get("version"), Some(&serde_json::json!("1")));
    }

    #[test]
    fn extension_round_trips_structured() {
        let info = sample();
        let mut extensions = Extensions::new();
        extensions.insert(
            ERC20_APPROVAL_GAS_SPONSORING_KEY,
            info.to_extension_entry().unwrap(),
        );
        let extracted = Erc20ApprovalGasSponsoringInfo::from_extensions(&extensions)
            .unwrap()
            .unwrap();
        assert_eq!(extracted, info);
    }

    #[test]
    fn missing_extension_ok_none() {
        assert!(
            Erc20ApprovalGasSponsoringInfo::from_extensions(&Extensions::new())
                .unwrap()
                .is_none()
        );
    }
}

#[cfg(test)]
#[cfg(feature = "facilitator")]
mod covering_tests {
    use std::str::FromStr;

    use alloy_consensus::{SignableTransaction, TxEip1559, TxEnvelope, TxLegacy};
    use alloy_eips::eip2718::Encodable2718;
    use alloy_eips::eip2930::AccessList;
    use alloy_network::TxSignerSync;
    use alloy_primitives::{Address, TxKind, U256, hex};
    use alloy_signer_local::PrivateKeySigner;
    use r402_protocol::error::{AsPaymentProblem, VerificationError};

    use super::*;
    use crate::permit2::PERMIT2_ADDRESS;

    fn anvil_signer() -> PrivateKeySigner {
        PrivateKeySigner::from_str(
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        )
        .expect("anvil key")
    }

    fn approve_calldata(spender: Address, amount: U256) -> Vec<u8> {
        let mut calldata = Vec::from(APPROVE_SELECTOR);
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(spender.as_slice());
        calldata.extend_from_slice(&amount.to_be_bytes::<32>());
        calldata
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "test fixture enumerates every signed-tx field"
    )]
    fn sign_eip1559(
        signer: &PrivateKeySigner,
        token: Address,
        spender: Address,
        amount: U256,
        chain_id: u64,
        nonce: u64,
        value: U256,
        max_fee_per_gas: u128,
        max_priority_fee_per_gas: u128,
    ) -> String {
        let mut tx = TxEip1559 {
            chain_id,
            nonce,
            gas_limit: 80_000,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to: TxKind::Call(token),
            value,
            access_list: AccessList::default(),
            input: approve_calldata(spender, amount).into(),
        };
        let sig = signer.sign_transaction_sync(&mut tx).expect("sign tx");
        let encoded = TxEnvelope::Eip1559(tx.into_signed(sig)).encoded_2718();
        format!("0x{}", hex::encode(encoded))
    }

    fn info(
        from: Address,
        token: Address,
        spender: Address,
        amount: U256,
        signed_transaction: String,
    ) -> Erc20ApprovalGasSponsoringInfo {
        Erc20ApprovalGasSponsoringInfo {
            from,
            asset: token,
            spender,
            amount: amount.to_string(),
            signed_transaction,
            version: "1".into(),
        }
    }

    fn wire(err: &VerificationError) -> String {
        err.as_payment_problem().reason().as_str().to_owned()
    }

    #[test]
    fn parse_failed_is_wire_code() {
        let payer = Address::repeat_byte(0x11);
        let token = Address::repeat_byte(0xAA);
        let payload = info(payer, token, PERMIT2_ADDRESS, U256::MAX, "0xdead".into());
        let err = decode_erc20_approval_structural(&payload, payer, token, 8453).unwrap_err();
        assert_eq!(wire(&err), "erc20_approval_tx_parse_failed");
    }

    #[test]
    fn chain_id_mismatch() {
        let signer = anvil_signer();
        let payer = signer.address();
        let token = Address::repeat_byte(0xAA);
        let signed = sign_eip1559(
            &signer,
            token,
            PERMIT2_ADDRESS,
            U256::MAX,
            1,
            0,
            U256::ZERO,
            1_000_000_000,
            1_000_000,
        );
        let payload = info(payer, token, PERMIT2_ADDRESS, U256::MAX, signed);
        let err = decode_erc20_approval_structural(&payload, payer, token, 8453).unwrap_err();
        assert_eq!(wire(&err), "erc20_approval_chain_id_mismatch");
    }

    #[test]
    fn nonce_mismatch() {
        let signer = anvil_signer();
        let payer = signer.address();
        let token = Address::repeat_byte(0xAA);
        let signed = sign_eip1559(
            &signer,
            token,
            PERMIT2_ADDRESS,
            U256::MAX,
            8453,
            7,
            U256::ZERO,
            1_000_000_000,
            1_000_000,
        );
        let payload = info(payer, token, PERMIT2_ADDRESS, U256::MAX, signed);
        let err =
            validate_erc20_approval_for_payment(&payload, payer, token, 8453, 0, 1).unwrap_err();
        assert_eq!(wire(&err), "erc20_approval_nonce_mismatch");
    }

    #[test]
    fn value_nonzero() {
        let signer = anvil_signer();
        let payer = signer.address();
        let token = Address::repeat_byte(0xAA);
        let signed = sign_eip1559(
            &signer,
            token,
            PERMIT2_ADDRESS,
            U256::MAX,
            8453,
            0,
            U256::from(1u64),
            1_000_000_000,
            1_000_000,
        );
        let payload = info(payer, token, PERMIT2_ADDRESS, U256::MAX, signed);
        let err = decode_erc20_approval_structural(&payload, payer, token, 8453).unwrap_err();
        assert_eq!(wire(&err), "erc20_approval_value_nonzero");
    }

    #[test]
    fn amount_mismatch() {
        let signer = anvil_signer();
        let payer = signer.address();
        let token = Address::repeat_byte(0xAA);
        let signed = sign_eip1559(
            &signer,
            token,
            PERMIT2_ADDRESS,
            U256::from(1u64),
            8453,
            0,
            U256::ZERO,
            1_000_000_000,
            1_000_000,
        );
        let payload = info(payer, token, PERMIT2_ADDRESS, U256::MAX, signed);
        let err = decode_erc20_approval_structural(&payload, payer, token, 8453).unwrap_err();
        assert_eq!(wire(&err), "erc20_approval_amount_mismatch");
    }

    #[test]
    fn not_eip1559() {
        let signer = anvil_signer();
        let payer = signer.address();
        let token = Address::repeat_byte(0xAA);
        let mut tx = TxLegacy {
            chain_id: Some(8453),
            nonce: 0,
            gas_price: 1_000_000_000,
            gas_limit: 80_000,
            to: TxKind::Call(token),
            value: U256::ZERO,
            input: approve_calldata(PERMIT2_ADDRESS, U256::MAX).into(),
        };
        let sig = signer.sign_transaction_sync(&mut tx).expect("sign");
        let signed = format!(
            "0x{}",
            hex::encode(TxEnvelope::Legacy(tx.into_signed(sig)).encoded_2718())
        );
        let payload = info(payer, token, PERMIT2_ADDRESS, U256::MAX, signed);
        let err = decode_erc20_approval_structural(&payload, payer, token, 8453).unwrap_err();
        assert_eq!(wire(&err), "erc20_approval_not_eip1559");
    }

    #[test]
    fn zero_priority_fee() {
        let signer = anvil_signer();
        let payer = signer.address();
        let token = Address::repeat_byte(0xAA);
        let signed = sign_eip1559(
            &signer,
            token,
            PERMIT2_ADDRESS,
            U256::MAX,
            8453,
            0,
            U256::ZERO,
            1_000_000_000,
            0,
        );
        let payload = info(payer, token, PERMIT2_ADDRESS, U256::MAX, signed);
        let err =
            validate_erc20_approval_for_payment(&payload, payer, token, 8453, 0, 1).unwrap_err();
        assert_eq!(wire(&err), "erc20_approval_fee_too_low");
    }

    #[test]
    fn deficit_cap_rejects_above_max() {
        let cost = U256::from(MAX_ERC20_SPONSOR_WEI) + U256::from(1u64);
        let err = funding_deficit(cost, U256::ZERO).unwrap_err();
        assert_eq!(wire(&err), "erc20_approval_funding_exceeds_cap");
    }

    #[test]
    fn self_funded_high_native_cost_is_ok() {
        let cost = U256::from(MAX_ERC20_SPONSOR_WEI) * U256::from(10u64);
        let deficit = funding_deficit(cost, cost).expect("self-funded is not capped");
        assert!(deficit.is_zero());
    }

    #[test]
    fn valid_max_approve_covers() {
        let signer = anvil_signer();
        let payer = signer.address();
        let token = Address::repeat_byte(0xAA);
        let signed = sign_eip1559(
            &signer,
            token,
            PERMIT2_ADDRESS,
            U256::MAX,
            8453,
            0,
            U256::ZERO,
            1_000_000_000,
            1_000_000,
        );
        let payload = info(payer, token, PERMIT2_ADDRESS, U256::MAX, signed);
        validate_erc20_approval_for_payment(&payload, payer, token, 8453, 0, 1)
            .expect("valid MAX approve");
    }

    #[test]
    fn wrong_target_is_rejected() {
        let signer = anvil_signer();
        let payer = signer.address();
        let token = Address::repeat_byte(0xAA);
        let other = Address::repeat_byte(0xBB);
        let signed = sign_eip1559(
            &signer,
            other,
            PERMIT2_ADDRESS,
            U256::MAX,
            8453,
            0,
            U256::ZERO,
            1_000_000_000,
            1_000_000,
        );
        let payload = info(payer, token, PERMIT2_ADDRESS, U256::MAX, signed);
        let err = decode_erc20_approval_structural(&payload, payer, token, 8453).unwrap_err();
        assert_eq!(wire(&err), "erc20_approval_tx_wrong_target");
    }
}
