//! TS `@x402/aptos` payload encoding and `aptos-sdk` BCS decode.
//!
//! Wire: base64(UTF-8 JSON `{ transaction: [u8], senderAuthenticator: [u8] }`).
//! Inner `transaction` is BCS of TS `SimpleTransaction` (raw txn + optional
//! fee-payer address). Inner `senderAuthenticator` is BCS
//! `AccountAuthenticator`. Construction/sign/verify use `aptos-sdk` 0.6.

use aptos_sdk::account::Account;
use aptos_sdk::crypto::{AnyPublicKey, AnySignature, MultiKeyPublicKey, MultiKeySignature};
use aptos_sdk::transaction::authenticator::AccountAuthenticator;
use aptos_sdk::transaction::payload::TransactionPayload;
use aptos_sdk::transaction::{
    EntryFunction, FeePayerRawTransaction, RawTransaction, SignedTransaction,
    TransactionAuthenticator,
};
use aptos_sdk::types::{AccountAddress, TypeTag};
use aptos_sdk::{AptosConfig, aptos_bcs};
use base64::Engine;
use serde::{Deserialize, Serialize};

/// TS `DecodedAptosPayload`: JSON arrays of BCS bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodedAptosPayload {
    /// BCS `SimpleTransaction` bytes.
    pub transaction: Vec<u8>,
    /// BCS `AccountAuthenticator` bytes.
    pub sender_authenticator: Vec<u8>,
}

/// TS `SimpleTransaction`: raw txn plus optional fee-payer address.
///
/// BCS layout is positional (`RawTransaction` then `Option<AccountAddress>`),
/// matching `@aptos-labs/ts-sdk` `SimpleTransaction.serialize`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimpleTransaction {
    /// Unsigned transaction body.
    pub raw_transaction: RawTransaction,
    /// Fee-payer address when sponsored; `None` when the sender pays gas.
    pub fee_payer_address: Option<AccountAddress>,
}

impl SimpleTransaction {
    /// Signing message the sender authenticator must cover.
    ///
    /// # Errors
    ///
    /// Returns [`AptosCodecError::Bcs`] if the SDK cannot serialize the
    /// signing domain.
    pub fn signing_message(&self) -> Result<Vec<u8>, AptosCodecError> {
        self.fee_payer_address.map_or_else(
            || {
                self.raw_transaction
                    .signing_message()
                    .map_err(|e| AptosCodecError::from_sdk(&e))
            },
            |fee_payer| {
                FeePayerRawTransaction::new_simple(self.raw_transaction.clone(), fee_payer)
                    .signing_message()
                    .map_err(|e| AptosCodecError::from_sdk(&e))
            },
        )
    }
}

/// Errors from payload encode/decode.
#[derive(Debug, thiserror::Error)]
pub enum AptosCodecError {
    /// Outer base64 or JSON-of-bytes is not the TS shape.
    #[error("invalid aptos payload encoding: {0}")]
    Encoding(String),
    /// Inner BCS could not be parsed with `aptos-sdk`.
    #[error("invalid aptos payload bcs: {0}")]
    Bcs(String),
}

impl AptosCodecError {
    fn from_sdk(err: &aptos_sdk::AptosError) -> Self {
        Self::Bcs(err.to_string())
    }
}

/// Encodes a simple transaction + sender authenticator as TS `encodeAptosPayload`.
///
/// # Errors
///
/// Returns [`AptosCodecError`] if BCS or JSON serialization fails.
pub fn encode_aptos_payload(
    transaction: &SimpleTransaction,
    sender_authenticator: &AccountAuthenticator,
) -> Result<String, AptosCodecError> {
    let transaction_bytes =
        aptos_bcs::to_bytes(transaction).map_err(|e| AptosCodecError::Bcs(e.to_string()))?;
    let authenticator_bytes = aptos_bcs::to_bytes(sender_authenticator)
        .map_err(|e| AptosCodecError::Bcs(e.to_string()))?;
    let decoded = DecodedAptosPayload {
        transaction: transaction_bytes,
        sender_authenticator: authenticator_bytes,
    };
    let json =
        serde_json::to_vec(&decoded).map_err(|e| AptosCodecError::Encoding(e.to_string()))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(json))
}

/// Decodes the TS outer envelope into JSON-of-bytes.
///
/// # Errors
///
/// Returns [`AptosCodecError::Encoding`] if base64 or JSON is invalid.
pub fn decode_aptos_payload(
    transaction_base64: &str,
) -> Result<DecodedAptosPayload, AptosCodecError> {
    let json = base64::engine::general_purpose::STANDARD
        .decode(transaction_base64.trim())
        .map_err(|e| AptosCodecError::Encoding(e.to_string()))?;
    serde_json::from_slice(&json).map_err(|e| AptosCodecError::Encoding(e.to_string()))
}

/// Fully decoded payment: simple txn + sender authenticator.
#[derive(Clone, Debug)]
pub struct DecodedAptosPayment {
    /// Deserialized simple transaction.
    pub transaction: SimpleTransaction,
    /// Deserialized sender authenticator.
    pub sender_authenticator: AccountAuthenticator,
}

impl DecodedAptosPayment {
    /// Decodes the x402 `payload.transaction` string.
    ///
    /// # Errors
    ///
    /// Returns [`AptosCodecError`] if the envelope or inner BCS is invalid.
    pub fn from_base64(transaction_base64: &str) -> Result<Self, AptosCodecError> {
        let decoded = decode_aptos_payload(transaction_base64)?;
        let transaction = aptos_bcs::from_bytes::<SimpleTransaction>(&decoded.transaction)
            .map_err(|e| AptosCodecError::Bcs(e.to_string()))?;
        let sender_authenticator =
            deserialize_account_authenticator(&decoded.sender_authenticator)?;
        Ok(Self {
            transaction,
            sender_authenticator,
        })
    }

    /// Sender long-form address (`0x` + 64 hex).
    #[must_use]
    pub fn sender_long(&self) -> String {
        self.transaction.raw_transaction.sender.to_long_string()
    }

    /// Entry-function payload when the txn is an entry function.
    #[must_use]
    pub const fn entry_function(&self) -> Option<&EntryFunction> {
        match &self.transaction.raw_transaction.payload {
            TransactionPayload::EntryFunction(entry) => Some(entry),
            _ => None,
        }
    }
}

/// Builds an `AccountAuthenticator` for `account` over `message`.
///
/// # Errors
///
/// Returns [`AptosCodecError`] if signing or key length is invalid.
pub fn account_authenticator_for(
    account: &impl Account,
    message: &[u8],
) -> Result<AccountAuthenticator, AptosCodecError> {
    let signature = account
        .sign(message)
        .map_err(|e| AptosCodecError::from_sdk(&e))?;
    let public_key = account.public_key_bytes();
    if public_key.len() != 32 || signature.len() != 64 {
        return Err(AptosCodecError::Bcs(
            "ed25519 authenticator requires 32-byte public key and 64-byte signature".to_owned(),
        ));
    }
    Ok(AccountAuthenticator::ed25519(public_key, signature))
}

/// Wraps a sender authenticator as a submit-ready [`TransactionAuthenticator`].
#[must_use]
pub fn transaction_authenticator_from_sender(
    sender: AccountAuthenticator,
    fee_payer: Option<AccountAddress>,
) -> TransactionAuthenticator {
    match fee_payer {
        Some(fee_payer_address) => TransactionAuthenticator::fee_payer(
            sender,
            Vec::new(),
            Vec::new(),
            fee_payer_address,
            AccountAuthenticator::no_account_authenticator(),
        ),
        None => match sender {
            AccountAuthenticator::Ed25519 {
                public_key,
                signature,
            } => TransactionAuthenticator::Ed25519 {
                public_key,
                signature,
            },
            AccountAuthenticator::MultiEd25519 {
                public_key,
                signature,
            } => TransactionAuthenticator::MultiEd25519 {
                public_key,
                signature,
            },
            other => TransactionAuthenticator::single_sender(other),
        },
    }
}

/// Signed transaction used for simulate/submit from a decoded payment.
#[must_use]
pub fn signed_from_decoded(decoded: &DecodedAptosPayment) -> SignedTransaction {
    SignedTransaction::new(
        decoded.transaction.raw_transaction.clone(),
        transaction_authenticator_from_sender(
            decoded.sender_authenticator.clone(),
            decoded.transaction.fee_payer_address,
        ),
    )
}

/// `aptos-sdk` client config for a CAIP-2 Aptos network.
///
/// # Errors
///
/// Returns [`AptosCodecError::Encoding`] if `rpc_url` is not a valid HTTP(S) URL.
pub fn aptos_config_for(
    chain_id: u8,
    rpc_url: Option<&str>,
) -> Result<AptosConfig, AptosCodecError> {
    if let Some(url) = rpc_url {
        return AptosConfig::custom(url).map_err(|e| AptosCodecError::Encoding(e.to_string()));
    }
    match chain_id {
        1 => Ok(AptosConfig::mainnet()),
        2 => Ok(AptosConfig::testnet()),
        _ => Err(AptosCodecError::Encoding(format!(
            "unsupported aptos chain id {chain_id}"
        ))),
    }
}

/// Returns `true` when `tag` is `0x1::fungible_asset::Metadata`.
#[must_use]
pub fn is_metadata_type_tag(tag: &TypeTag) -> bool {
    match tag {
        TypeTag::Struct(st) => {
            st.address == AccountAddress::ONE
                && st.module.as_str() == "fungible_asset"
                && st.name.as_str() == "Metadata"
                && st.type_args.is_empty()
        }
        _ => false,
    }
}

/// Returns `true` when the entry function is a supported FA transfer.
#[must_use]
pub fn is_supported_transfer(entry: &EntryFunction) -> bool {
    if entry.module.address != AccountAddress::ONE || entry.function != "transfer" {
        return false;
    }
    let module = entry.module.name.as_str();
    module == "primary_fungible_store" || module == "fungible_asset"
}

fn deserialize_account_authenticator(
    bytes: &[u8],
) -> Result<AccountAuthenticator, AptosCodecError> {
    if let Ok(auth) = aptos_bcs::from_bytes::<AccountAuthenticator>(bytes) {
        return Ok(auth);
    }
    // On-chain SingleKey/MultiKey bytes differ from SDK Deserialize; this parser is the product.
    let tag = bytes
        .first()
        .copied()
        .ok_or_else(|| AptosCodecError::Bcs("empty account authenticator".to_owned()))?;
    match tag {
        2 => {
            let rest = bytes.get(1..).ok_or_else(|| {
                AptosCodecError::Bcs("truncated SingleKey authenticator".to_owned())
            })?;
            let (pk, pk_len) = take_any_public_key(rest)?;
            let sig_bytes = rest
                .get(pk_len..)
                .ok_or_else(|| AptosCodecError::Bcs("truncated SingleKey signature".to_owned()))?;
            let sig = AnySignature::from_bcs_bytes(sig_bytes)
                .map_err(|e| AptosCodecError::from_sdk(&e))?;
            Ok(AccountAuthenticator::single_key(
                pk.to_bcs_bytes(),
                sig.to_bcs_bytes(),
            ))
        }
        3 => {
            let rest = bytes.get(1..).ok_or_else(|| {
                AptosCodecError::Bcs("truncated MultiKey authenticator".to_owned())
            })?;
            let (pk, pk_len) = take_multi_key_public(rest)?;
            let sig_bytes = rest
                .get(pk_len..)
                .ok_or_else(|| AptosCodecError::Bcs("truncated MultiKey signature".to_owned()))?;
            let sig = MultiKeySignature::from_bytes(sig_bytes)
                .map_err(|e| AptosCodecError::from_sdk(&e))?;
            Ok(AccountAuthenticator::multi_key(
                pk.to_bytes(),
                sig.to_bytes(),
            ))
        }
        _ => Err(AptosCodecError::Bcs(format!(
            "unsupported account authenticator variant {tag}"
        ))),
    }
}

/// `AnyPublicKey` on-chain: `variant || ULEB128(len) || bytes`.
fn take_any_public_key(bytes: &[u8]) -> Result<(AnyPublicKey, usize), AptosCodecError> {
    let len = any_public_key_declared_len(bytes)?;
    let prefix = bytes
        .get(..len)
        .ok_or_else(|| AptosCodecError::Bcs("truncated AnyPublicKey".to_owned()))?;
    let pk = AnyPublicKey::from_bcs_bytes(prefix).map_err(|e| AptosCodecError::from_sdk(&e))?;
    Ok((pk, len))
}

fn any_public_key_declared_len(bytes: &[u8]) -> Result<usize, AptosCodecError> {
    let rest = bytes
        .get(1..)
        .ok_or_else(|| AptosCodecError::Bcs("truncated AnyPublicKey".to_owned()))?;
    let (payload_len, prefix_len) = uleb128_len(rest)?;
    1usize
        .checked_add(prefix_len)
        .and_then(|n| n.checked_add(payload_len))
        .ok_or_else(|| AptosCodecError::Bcs("AnyPublicKey length overflow".to_owned()))
}

/// `MultiKeyPublicKey` on-chain: `num_keys || AnyPublicKey* || threshold`.
fn take_multi_key_public(bytes: &[u8]) -> Result<(MultiKeyPublicKey, usize), AptosCodecError> {
    let num_keys = usize::from(
        *bytes
            .first()
            .ok_or_else(|| AptosCodecError::Bcs("truncated MultiKeyPublicKey".to_owned()))?,
    );
    if num_keys == 0 || num_keys > 32 {
        return Err(AptosCodecError::Bcs(format!(
            "invalid MultiKeyPublicKey key count {num_keys}"
        )));
    }
    let mut offset = 1usize;
    for _ in 0..num_keys {
        let used =
            any_public_key_declared_len(bytes.get(offset..).ok_or_else(|| {
                AptosCodecError::Bcs("truncated MultiKeyPublicKey key".to_owned())
            })?)?;
        offset = offset
            .checked_add(used)
            .ok_or_else(|| AptosCodecError::Bcs("MultiKeyPublicKey offset overflow".to_owned()))?;
    }
    offset = offset
        .checked_add(1)
        .ok_or_else(|| AptosCodecError::Bcs("truncated MultiKeyPublicKey threshold".to_owned()))?;
    let prefix = bytes
        .get(..offset)
        .ok_or_else(|| AptosCodecError::Bcs("truncated MultiKeyPublicKey".to_owned()))?;
    let pk = MultiKeyPublicKey::from_bytes(prefix).map_err(|e| AptosCodecError::from_sdk(&e))?;
    Ok((pk, offset))
}

fn uleb128_len(bytes: &[u8]) -> Result<(usize, usize), AptosCodecError> {
    let mut value = 0usize;
    let mut shift = 0;
    for (i, &byte) in bytes.iter().enumerate() {
        if i >= 5 {
            return Err(AptosCodecError::Bcs("ULEB128 too long".to_owned()));
        }
        value |= usize::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, i + 1));
        }
        shift += 7;
    }
    Err(AptosCodecError::Bcs("truncated ULEB128".to_owned()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use aptos_sdk::account::Ed25519Account;
    use aptos_sdk::transaction::InputEntryFunctionData;
    use aptos_sdk::transaction::builder::TransactionBuilder;
    use aptos_sdk::types::ChainId;

    use super::*;
    use crate::chain::{DEFAULT_CLIENT_MAX_GAS, USDC_TESTNET_FA};

    fn sample_payment() -> (SimpleTransaction, AccountAuthenticator) {
        let sender = Ed25519Account::generate();
        let fee_payer = Ed25519Account::generate();
        let pay_to = AccountAddress::ONE;
        let asset = AccountAddress::from_hex(USDC_TESTNET_FA).unwrap();
        let payload = InputEntryFunctionData::transfer_fungible_asset(asset, pay_to, 1000).unwrap();
        let raw = TransactionBuilder::new()
            .sender(sender.address())
            .sequence_number(0)
            .payload(payload)
            .max_gas_amount(DEFAULT_CLIENT_MAX_GAS)
            .gas_unit_price(100)
            .expiration_timestamp_secs(1_900_000_000)
            .chain_id(ChainId::testnet())
            .build()
            .unwrap();
        let tx = SimpleTransaction {
            raw_transaction: raw,
            fee_payer_address: Some(fee_payer.address()),
        };
        let message = tx.signing_message().unwrap();
        let auth = account_authenticator_for(&sender, &message).unwrap();
        (tx, auth)
    }

    #[test]
    fn encode_decode_roundtrip_matches_ts_json_shape() {
        let (tx, auth) = sample_payment();
        let encoded = encode_aptos_payload(&tx, &auth).unwrap();
        let decoded_json = decode_aptos_payload(&encoded).unwrap();
        assert!(!decoded_json.transaction.is_empty());
        assert!(!decoded_json.sender_authenticator.is_empty());

        let json = base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert!(
            value
                .get("transaction")
                .and_then(serde_json::Value::as_array)
                .is_some()
        );
        assert!(
            value
                .get("senderAuthenticator")
                .and_then(serde_json::Value::as_array)
                .is_some()
        );

        let payment = DecodedAptosPayment::from_base64(&encoded).unwrap();
        assert_eq!(payment.transaction, tx);
        assert_eq!(
            payment.transaction.raw_transaction.chain_id,
            ChainId::testnet()
        );
        let entry = payment.entry_function().unwrap();
        assert!(is_supported_transfer(entry));
        assert_eq!(entry.args.len(), 3);
        let asset: AccountAddress = aptos_bcs::from_bytes(entry.args.first().unwrap()).unwrap();
        assert_eq!(asset, AccountAddress::from_hex(USDC_TESTNET_FA).unwrap());
        payment
            .sender_authenticator
            .verify(&tx.signing_message().unwrap())
            .unwrap();
    }

    fn ts_fixture(name: &str) -> (String, serde_json::Value) {
        let root: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/aptos/ts_encode_aptos_payload.json"
        ))
        .unwrap();
        let entry = root.get(name).unwrap();
        (
            entry
                .get("encoded")
                .and_then(serde_json::Value::as_str)
                .unwrap()
                .to_owned(),
            entry.clone(),
        )
    }

    fn assert_ts_payment(name: &str, expected_variant: fn(&AccountAuthenticator) -> bool) {
        let (encoded, meta) = ts_fixture(name);
        let payment = DecodedAptosPayment::from_base64(&encoded).unwrap();
        assert!(
            expected_variant(&payment.sender_authenticator),
            "{name} authenticator variant"
        );
        assert_eq!(
            payment.sender_long(),
            meta.get("sender")
                .and_then(serde_json::Value::as_str)
                .unwrap()
        );
        assert_eq!(
            payment
                .transaction
                .fee_payer_address
                .unwrap()
                .to_long_string(),
            meta.get("feePayer")
                .and_then(serde_json::Value::as_str)
                .unwrap()
        );
        let entry = payment.entry_function().unwrap();
        assert!(is_supported_transfer(entry));
        let asset: AccountAddress = aptos_bcs::from_bytes(entry.args.first().unwrap()).unwrap();
        assert_eq!(
            asset.to_long_string(),
            meta.get("asset")
                .and_then(serde_json::Value::as_str)
                .unwrap()
        );
        let pay_to: AccountAddress = aptos_bcs::from_bytes(entry.args.get(1).unwrap()).unwrap();
        assert_eq!(
            pay_to.to_long_string(),
            meta.get("payTo")
                .and_then(serde_json::Value::as_str)
                .unwrap()
        );
        let amount: u64 = aptos_bcs::from_bytes(entry.args.get(2).unwrap()).unwrap();
        assert_eq!(
            amount.to_string(),
            meta.get("amount")
                .and_then(serde_json::Value::as_str)
                .unwrap()
        );
        payment
            .sender_authenticator
            .verify(&payment.transaction.signing_message().unwrap())
            .unwrap();
    }

    #[test]
    fn decodes_ts_encode_aptos_payload_ed25519() {
        assert_ts_payment("ed25519", |a| {
            matches!(a, AccountAuthenticator::Ed25519 { .. })
        });
    }

    #[test]
    fn decodes_ts_encode_aptos_payload_single_key_ed25519() {
        assert_ts_payment("singleKeyEd25519", |a| {
            matches!(a, AccountAuthenticator::SingleKey { .. })
        });
    }

    #[test]
    fn decodes_ts_encode_aptos_payload_single_key_secp256k1() {
        assert_ts_payment("singleKeySecp256k1", |a| {
            matches!(a, AccountAuthenticator::SingleKey { .. })
        });
    }

    #[test]
    fn decodes_ts_encode_aptos_payload_multi_key() {
        assert_ts_payment("multiKey", |a| {
            matches!(a, AccountAuthenticator::MultiKey { .. })
        });
    }
}
