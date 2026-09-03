//! Build and convert Concordium V1 sponsored transactions.

use std::collections::BTreeMap;
use std::str::FromStr;

use concordium_rust_sdk::base::base::Energy;
use concordium_rust_sdk::base::common::cbor::cbor_decode;
use concordium_rust_sdk::base::common::types::{
    Amount, CredentialIndex, KeyIndex, Signature, TransactionSignature, TransactionSignaturesV1,
    TransactionTime,
};
use concordium_rust_sdk::base::contracts_common::AccountAddress;
use concordium_rust_sdk::base::hashes::TransactionSignHash;
use concordium_rust_sdk::base::protocol_level_tokens::{
    RawCbor, TokenAmount, TokenId, TokenOperation, TokenOperations, TokenOperationsPayload,
    operations,
};
use concordium_rust_sdk::base::transactions::{
    AccountTransactionV1, EncodedPayload, HasAccountAccessStructure, Memo, Payload, PayloadLike,
    TransactionHeaderV1, compute_transaction_sign_hash_v1, construct, cost,
    verify_signature_transaction_sign_hash_v1,
};
use concordium_rust_sdk::types::Nonce;
use r402_protocol::error::ClientError;
use serde_json::Value;

use super::signer::ConcordiumSigner;
use crate::exact::payload::{
    CredentialSignatureMap, SignableV1Signatures, SignableV1Transaction,
    SignableV1TransactionHeader, SignableV1TransactionPayload, SponsorHeader,
};

/// Decoded transfer used by facilitator checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedPayload {
    /// Recipient Base58, if present.
    pub recipient: Option<String>,
    /// Atomic amount.
    pub amount: Option<u128>,
    /// PLT token id.
    pub token_id: Option<String>,
    /// PLT decimals encoded in the amount.
    pub token_decimals: Option<u8>,
}

/// Builds a sender-signed V1 CCD transfer with an empty sponsor signature slot.
///
/// # Errors
///
/// Invalid addresses, amount, or signing.
pub fn build_ccd_transfer(
    signer: &ConcordiumSigner,
    pay_to: &str,
    amount: &str,
    fee_payer: &str,
    nonce: u64,
    expiry_unix: u64,
) -> Result<SignableV1Transaction, ClientError> {
    let sender = sdk_address(&signer.address().to_string())?;
    let receiver = sdk_address(pay_to)?;
    let sponsor = sdk_address(fee_payer)?;
    let micro = parse_u64_amount(amount)?;
    let mut pre = construct::transfer(
        1,
        sender,
        Nonce { nonce },
        TransactionTime::from_seconds(expiry_unix),
        receiver,
        Amount::from_micro_ccd(micro),
    )
    .extend();
    pre.add_sponsor(sponsor, 1).map_err(ClientError::Signing)?;
    let _ = pre.sign(signer);
    pre_to_wire(
        &pre,
        SignableV1TransactionPayload::Transfer {
            to_address: Some(pay_to.to_owned()),
            amount: Value::String(amount.to_owned()),
        },
        cost::SIMPLE_TRANSFER.energy,
    )
}

/// Builds a sender-signed V1 PLT transfer with an empty sponsor signature slot.
///
/// # Errors
///
/// Invalid addresses, token id, amount, or signing.
#[allow(
    clippy::too_many_arguments,
    reason = "mirrors official client PLT builder parameters"
)]
pub fn build_plt_transfer(
    signer: &ConcordiumSigner,
    pay_to: &str,
    amount: &str,
    token_id: &str,
    decimals: u8,
    fee_payer: &str,
    nonce: u64,
    expiry_unix: u64,
) -> Result<SignableV1Transaction, ClientError> {
    let sender = sdk_address(&signer.address().to_string())?;
    let receiver = sdk_address(pay_to)?;
    let sponsor = sdk_address(fee_payer)?;
    let raw = parse_u64_amount(amount)?;
    let token: TokenId = token_id
        .parse()
        .map_err(|e| ClientError::Signing(format!("{e}")))?;
    let op = operations::transfer_tokens(receiver, TokenAmount::from_raw(raw, decimals));
    let mut pre = construct::token_update_operations(
        1,
        sender,
        Nonce { nonce },
        TransactionTime::from_seconds(expiry_unix),
        token,
        std::iter::once(op).collect(),
    )
    .map_err(|e| ClientError::Signing(format!("{e}")))?
    .extend();
    pre.add_sponsor(sponsor, 1).map_err(ClientError::Signing)?;
    let _ = pre.sign(signer);
    let operations_hex = match &pre.payload {
        Payload::TokenUpdate { payload } => hex::encode(payload.operations.as_ref()),
        _ => String::new(),
    };
    pre_to_wire(
        &pre,
        SignableV1TransactionPayload::TokenUpdate {
            token_id: Some(token_id.to_owned()),
            operations: Value::String(operations_hex),
        },
        (cost::PLT_OPERATIONS_TRANSACTIONS + cost::PLT_TRANSFER).energy,
    )
}

/// Rebuilds an SDK V1 transaction from wire JSON for signature verification.
///
/// # Errors
///
/// Missing fields or invalid addresses/amounts.
pub fn rebuild_for_verify(
    tx: &SignableV1Transaction,
) -> Result<AccountTransactionV1<EncodedPayload>, ClientError> {
    let pre = rebuild_pre(tx)?;
    let sender_sig = signature_from_map(&tx.signatures.sender)?;
    Ok(AccountTransactionV1 {
        signatures: TransactionSignaturesV1 {
            sender: sender_sig,
            sponsor: None,
        },
        header: pre.header.clone(),
        payload: pre.encoded,
    })
}

/// Adds a sponsor signature and finalizes.
///
/// # Errors
///
/// Rebuild or sponsor signing failed.
pub fn add_sponsor_signature(
    tx: &SignableV1Transaction,
    sponsor: &ConcordiumSigner,
) -> Result<AccountTransactionV1<EncodedPayload>, ClientError> {
    let mut pre = rebuild_pre(tx)?;
    let sender_sig = signature_from_map(&tx.signatures.sender)?;
    pre.sender_signature = Some(sender_sig);
    pre.sponsor(sponsor).map_err(ClientError::Signing)?;
    pre.finalize().map_err(ClientError::Signing)
}

/// Verifies the sender signature against `keys`.
#[must_use]
pub fn verify_sender_signature<K: HasAccountAccessStructure>(
    rebuilt: &AccountTransactionV1<EncodedPayload>,
    keys: &K,
) -> bool {
    let hash = rebuilt.hash_to_sign_ref();
    verify_signature_transaction_sign_hash_v1(keys, keys, &hash, &rebuilt.signatures)
}

/// Hash used for signing a rebuilt transaction.
pub trait HashToSign {
    /// Transaction sign hash.
    fn hash_to_sign_ref(&self) -> TransactionSignHash;
}

impl HashToSign for AccountTransactionV1<EncodedPayload> {
    fn hash_to_sign_ref(&self) -> TransactionSignHash {
        compute_transaction_sign_hash_v1(&self.header, &self.payload)
    }
}

/// Decodes CCD / PLT payload details.
///
/// # Errors
///
/// Official `invalidReason` string when the payload is not a single transfer.
pub fn decode_payload(payload: &SignableV1TransactionPayload) -> Result<DecodedPayload, String> {
    match payload {
        SignableV1TransactionPayload::Transfer { to_address, amount }
        | SignableV1TransactionPayload::TransferWithMemo {
            to_address, amount, ..
        } => {
            let Some(to) = to_address.as_ref().filter(|s| !s.is_empty()) else {
                return Err("missing_recipient".to_owned());
            };
            Ok(DecodedPayload {
                recipient: Some(to.clone()),
                amount: Some(parse_amount_value(amount)?),
                token_id: None,
                token_decimals: None,
            })
        }
        SignableV1TransactionPayload::TokenUpdate {
            token_id,
            operations,
        } => decode_token_update(token_id.as_deref(), operations),
    }
}

fn decode_token_update(
    token_id: Option<&str>,
    operations: &Value,
) -> Result<DecodedPayload, String> {
    let Some(token_id) = token_id.filter(|s| !s.is_empty()) else {
        return Err("missing_token_id".to_owned());
    };
    if operations.is_null() {
        return Err("missing_token_operations".to_owned());
    }
    let ops = decode_token_operations(operations)?;
    if ops.operations.len() != 1 {
        return Err("unexpected_token_operations_count".to_owned());
    }
    let Some(op) = ops.operations.first() else {
        return Err("unexpected_token_operations_count".to_owned());
    };
    let concordium_rust_sdk::base::common::upward::CborUpward::Known(TokenOperation::Transfer(
        transfer,
    )) = op
    else {
        return Err("unexpected_token_operation".to_owned());
    };
    Ok(DecodedPayload {
        recipient: Some(transfer.recipient.address.to_string()),
        amount: Some(u128::from(transfer.amount.value())),
        token_id: Some(token_id.to_owned()),
        token_decimals: Some(transfer.amount.decimals()),
    })
}

fn decode_token_operations(operations: &Value) -> Result<TokenOperations, String> {
    let bytes = match operations {
        Value::String(s) if !s.is_empty() => hex::decode(s.trim_start_matches("0x"))
            .map_err(|_| "invalid_token_operations".to_owned())?,
        _ => return Err("invalid_token_operations".to_owned()),
    };
    cbor_decode(&bytes).map_err(|_| "invalid_token_operations".to_owned())
}

fn rebuild_pre(
    tx: &SignableV1Transaction,
) -> Result<construct::PreAccountTransactionV1, ClientError> {
    let sender = sdk_address(&tx.header.sender)?;
    let sponsor = tx
        .header
        .sponsor
        .as_ref()
        .and_then(SponsorHeader::resolved_address)
        .ok_or_else(|| ClientError::Signing("missing sponsor".to_owned()))?;
    let sponsor = sdk_address(sponsor)?;
    let expiry = expiry_unix(&tx.header.expiry)?;
    let sender_sigs = u32::try_from(tx.header.num_signatures.max(1)).unwrap_or(1);
    let sponsor_sigs = tx
        .header
        .sponsor
        .as_ref()
        .map_or(1, |s| u32::try_from(s.num_signatures.max(1)).unwrap_or(1));
    let (payload, encoded) = encode_wire_payload(&tx.payload)?;
    let base = tx
        .header
        .execution_energy_amount
        .unwrap_or_else(|| infer_base_energy(&tx.payload));
    let payload_size = u64::from(u32::from(encoded.size()));
    let header = TransactionHeaderV1 {
        sender,
        nonce: Nonce {
            nonce: tx.header.nonce,
        },
        energy_amount: sponsored_v1_energy(base, payload_size, sender_sigs, sponsor_sigs),
        payload_size: encoded.size(),
        expiry: TransactionTime::from_seconds(expiry),
        sponsor: Some(sponsor),
    };
    let hash_to_sign = compute_transaction_sign_hash_v1(&header, &encoded);
    Ok(construct::PreAccountTransactionV1 {
        header,
        payload,
        encoded,
        hash_to_sign,
        sender_signature: None,
        sponsor_signature: None,
    })
}

/// Official web-sdk JSON `executionEnergyAmount` is **base** NRG (size/sig
/// costs excluded). `preFinalized` then adds A×sigs + B×(header+payload).
fn infer_base_energy(payload: &SignableV1TransactionPayload) -> u64 {
    match payload {
        SignableV1TransactionPayload::Transfer { .. }
        | SignableV1TransactionPayload::TransferWithMemo { .. } => cost::SIMPLE_TRANSFER.energy,
        SignableV1TransactionPayload::TokenUpdate { operations, .. } => {
            plt_base_energy_from_operations(operations)
        }
    }
}

fn plt_base_energy_from_operations(operations: &Value) -> u64 {
    let Ok(ops) = decode_token_operations(operations) else {
        return (cost::PLT_OPERATIONS_TRANSACTIONS + cost::PLT_TRANSFER).energy;
    };
    let mut energy = cost::PLT_OPERATIONS_TRANSACTIONS.energy;
    for op in &ops.operations {
        energy += match op {
            concordium_rust_sdk::base::common::upward::CborUpward::Known(
                TokenOperation::Transfer(_),
            ) => cost::PLT_TRANSFER.energy,
            concordium_rust_sdk::base::common::upward::CborUpward::Known(
                TokenOperation::Mint(_) | TokenOperation::Burn(_),
            ) => cost::PLT_MINT.energy,
            concordium_rust_sdk::base::common::upward::CborUpward::Known(
                TokenOperation::AddAllowList(_)
                | TokenOperation::RemoveAllowList(_)
                | TokenOperation::AddDenyList(_)
                | TokenOperation::RemoveDenyList(_),
            ) => cost::PLT_LIST_UPDATE.energy,
            concordium_rust_sdk::base::common::upward::CborUpward::Known(
                TokenOperation::Pause(_) | TokenOperation::Unpause(_),
            ) => cost::PLT_PAUSE.energy,
            concordium_rust_sdk::base::common::upward::CborUpward::Unknown(_) => 0,
        };
    }
    energy
}

/// `AccountTransactionV1.calculateEnergyCost` with a sponsor present:
/// A×(sender+sponsor sigs) + B×(bitmap + v0 header + 32 + payload) + base.
fn sponsored_v1_energy(
    base: u64,
    payload_size: u64,
    sender_sigs: u32,
    sponsor_sigs: u32,
) -> Energy {
    const BITMAP_AND_SPONSOR: u64 = 2 + 32;
    Energy::from(
        cost::A * u64::from(sender_sigs.saturating_add(sponsor_sigs))
            + cost::B * (BITMAP_AND_SPONSOR + construct::TRANSACTION_HEADER_SIZE + payload_size)
            + base,
    )
}

/// Encode the wire payload. PLT `operations` hex is used as-is (no CBOR
/// re-encode) so the sign hash matches official `Payload.fromJSON`.
fn encode_wire_payload(
    payload: &SignableV1TransactionPayload,
) -> Result<(Payload, EncodedPayload), ClientError> {
    let structured = match payload {
        SignableV1TransactionPayload::Transfer { to_address, amount } => {
            let to = to_address
                .as_deref()
                .ok_or_else(|| ClientError::Signing("missing recipient".to_owned()))?;
            Payload::Transfer {
                to_address: sdk_address(to)?,
                amount: Amount::from_micro_ccd(parse_u64_amount(&amount_string(amount))?),
            }
        }
        SignableV1TransactionPayload::TransferWithMemo {
            to_address,
            amount,
            memo,
        } => {
            let to = to_address
                .as_deref()
                .ok_or_else(|| ClientError::Signing("missing recipient".to_owned()))?;
            let memo_bytes = match memo.as_deref() {
                Some(hex_str) if !hex_str.is_empty() => {
                    hex::decode(hex_str.trim_start_matches("0x"))
                        .map_err(|e| ClientError::Signing(format!("invalid memo hex: {e}")))?
                }
                _ => Vec::new(),
            };
            let parsed_memo = Memo::try_from(memo_bytes)
                .map_err(|_| ClientError::Signing("memo too big".to_owned()))?;
            Payload::TransferWithMemo {
                to_address: sdk_address(to)?,
                memo: parsed_memo,
                amount: Amount::from_micro_ccd(parse_u64_amount(&amount_string(amount))?),
            }
        }
        SignableV1TransactionPayload::TokenUpdate {
            token_id,
            operations,
        } => {
            let token_id = token_id
                .as_deref()
                .ok_or_else(|| ClientError::Signing("missing token id".to_owned()))?;
            let token: TokenId = token_id
                .parse()
                .map_err(|e| ClientError::Signing(format!("{e}")))?;
            let bytes = match operations {
                Value::String(s) if !s.is_empty() => hex::decode(s.trim_start_matches("0x"))
                    .map_err(|_| ClientError::Signing("invalid_token_operations".to_owned()))?,
                _ => {
                    return Err(ClientError::Signing("invalid_token_operations".to_owned()));
                }
            };
            Payload::TokenUpdate {
                payload: TokenOperationsPayload {
                    token_id: token,
                    operations: RawCbor::from(bytes),
                },
            }
        }
    };
    let encoded = structured.encode();
    Ok((structured, encoded))
}

fn pre_to_wire(
    pre: &construct::PreAccountTransactionV1,
    payload: SignableV1TransactionPayload,
    base_energy: u64,
) -> Result<SignableV1Transaction, ClientError> {
    let sender_map = signature_to_map(
        pre.sender_signature
            .as_ref()
            .ok_or_else(|| ClientError::Signing("missing sender signature".to_owned()))?,
    );
    let sponsor_addr = pre
        .header
        .sponsor
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    Ok(SignableV1Transaction {
        version: 1,
        header: SignableV1TransactionHeader {
            sender: pre.header.sender.to_string(),
            nonce: pre.header.nonce.nonce,
            expiry: Value::from(pre.header.expiry.seconds),
            num_signatures: 1,
            execution_energy_amount: Some(base_energy),
            sponsor: Some(SponsorHeader {
                account: Some(sponsor_addr),
                address: None,
                num_signatures: 1,
            }),
        },
        payload,
        signatures: SignableV1Signatures {
            sender: sender_map,
            sponsor: BTreeMap::new(),
        },
    })
}

fn sdk_address(s: &str) -> Result<AccountAddress, ClientError> {
    AccountAddress::from_str(s).map_err(|e| ClientError::Signing(format!("{e}")))
}

fn parse_u64_amount(s: &str) -> Result<u64, ClientError> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ClientError::Signing(
            "amount must be a non-empty decimal string".to_owned(),
        ));
    }
    s.parse()
        .map_err(|e| ClientError::Signing(format!("invalid amount: {e}")))
}

fn amount_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn parse_amount_value(value: &Value) -> Result<u128, String> {
    let s = amount_string(value);
    if s.is_empty() {
        return Err("invalid_amount_format".to_owned());
    }
    s.parse().map_err(|_| "invalid_amount_format".to_owned())
}

pub(crate) fn expiry_unix(value: &Value) -> Result<u64, ClientError> {
    match value {
        Value::Number(n) => n
            .as_u64()
            .ok_or_else(|| ClientError::Signing("invalid expiry".to_owned())),
        Value::String(s) => s
            .parse()
            .map_err(|_| ClientError::Signing("invalid expiry".to_owned())),
        _ => Err(ClientError::Signing("invalid expiry".to_owned())),
    }
}

fn signature_to_map(sig: &TransactionSignature) -> CredentialSignatureMap {
    let mut out = CredentialSignatureMap::new();
    for (ci, keys) in &sig.signatures {
        let mut inner = BTreeMap::new();
        for (ki, signature) in keys {
            let _ = inner.insert(ki.0.to_string(), hex::encode(&signature.sig));
        }
        let _ = out.insert(ci.index.to_string(), inner);
    }
    out
}

fn signature_from_map(map: &CredentialSignatureMap) -> Result<TransactionSignature, ClientError> {
    let mut signatures = BTreeMap::new();
    for (ci, keys) in map {
        let cred: u8 = ci
            .parse()
            .map_err(|_| ClientError::Signing("invalid credential index".to_owned()))?;
        let mut inner = BTreeMap::new();
        for (ki, hex_sig) in keys {
            let key: u8 = ki
                .parse()
                .map_err(|_| ClientError::Signing("invalid key index".to_owned()))?;
            let bytes = hex::decode(hex_sig)
                .map_err(|e| ClientError::Signing(format!("invalid signature hex: {e}")))?;
            let _ = inner.insert(KeyIndex(key), Signature { sig: bytes });
        }
        let _ = signatures.insert(CredentialIndex { index: cred }, inner);
    }
    Ok(TransactionSignature { signatures })
}

/// Whether the sender map contains at least one non-empty signature string.
#[must_use]
pub fn has_sender_signature(map: &CredentialSignatureMap) -> bool {
    map.values()
        .any(|keys| keys.values().any(|sig| !sig.is_empty()))
}
