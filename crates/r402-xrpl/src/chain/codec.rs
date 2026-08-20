//! Binary codec, signing, and amount helpers used by client and facilitator.

use std::cmp::Ordering;

use serde_json::Value;
use sha2::{Digest, Sha256, Sha512};
use xrpl::core::binarycodec::{decode, encode, encode_for_signing};
use xrpl::core::keypairs::{is_valid_message, sign};

use crate::chain::types::is_decimal_string;
use crate::{
    CANONICAL_SIGNING_PUB_KEY_LEN, DEFAULT_LEDGER_CLOSE_SECONDS, DEFAULT_LEDGER_TOLERANCE,
    TRANSACTION_HASH_PREFIX,
};

/// Errors from blob decode, encode, or signature operations.
#[derive(Debug, thiserror::Error)]
pub enum XrplCodecError {
    /// Hex or binary codec failure.
    #[error("xrpl codec error: {0}")]
    Codec(String),
    /// Signature creation failed.
    #[error("xrpl signing error: {0}")]
    Sign(String),
}

/// Decodes a hex-encoded signed transaction blob to JSON.
///
/// # Errors
///
/// Returns [`XrplCodecError::Codec`] when the blob is not hex or is not an `STObject`.
pub fn decode_signed_tx_blob(signed_tx_blob: &str) -> Result<Value, XrplCodecError> {
    if signed_tx_blob.is_empty() || !signed_tx_blob.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(XrplCodecError::Codec("signedTxBlob must be hex".to_owned()));
    }
    decode(signed_tx_blob).map_err(|e| XrplCodecError::Codec(e.to_string()))
}

/// Encodes a transaction JSON object to a hex blob.
///
/// # Errors
///
/// Returns [`XrplCodecError::Codec`] when serialization fails.
pub fn encode_tx(tx: &Value) -> Result<String, XrplCodecError> {
    encode(tx).map_err(|e| XrplCodecError::Codec(e.to_string()))
}

/// Signs a prepared transaction object and returns the hex-encoded signed blob.
///
/// Sets `SigningPubKey` and `TxnSignature` on `tx`.
///
/// # Errors
///
/// Returns [`XrplCodecError`] when encoding or signing fails.
pub fn sign_transaction(
    tx: &mut Value,
    private_key_hex: &str,
    public_key_hex: &str,
) -> Result<String, XrplCodecError> {
    let Some(obj) = tx.as_object_mut() else {
        return Err(XrplCodecError::Codec(
            "transaction must be a JSON object".to_owned(),
        ));
    };
    obj.insert(
        "SigningPubKey".to_owned(),
        Value::String(public_key_hex.to_owned()),
    );
    let _ = obj.remove("TxnSignature");
    let signing_hex = encode_for_signing(tx).map_err(|e| XrplCodecError::Codec(e.to_string()))?;
    let signing_bytes =
        hex::decode(&signing_hex).map_err(|e| XrplCodecError::Codec(e.to_string()))?;
    let signature =
        sign(&signing_bytes, private_key_hex).map_err(|e| XrplCodecError::Sign(e.to_string()))?;
    let Some(signed) = tx.as_object_mut() else {
        return Err(XrplCodecError::Codec(
            "transaction must be a JSON object".to_owned(),
        ));
    };
    signed.insert("TxnSignature".to_owned(), Value::String(signature));
    encode_tx(tx)
}

/// Verifies that `signed_tx_blob` was signed by its embedded `SigningPubKey`.
#[must_use]
pub fn verify_tx_signature(_signed_tx_blob: &str, decoded: &Value) -> bool {
    let Some(public_key) = decoded.get("SigningPubKey").and_then(Value::as_str) else {
        return false;
    };
    let Some(signature) = decoded.get("TxnSignature").and_then(Value::as_str) else {
        return false;
    };
    let mut unsigned = decoded.clone();
    if let Some(obj) = unsigned.as_object_mut() {
        let _ = obj.remove("TxnSignature");
        let _ = obj.remove("Signers");
    }
    let Ok(signing_hex) = encode_for_signing(&unsigned) else {
        return false;
    };
    let Ok(signing_bytes) = hex::decode(&signing_hex) else {
        return false;
    };
    is_valid_message(&signing_bytes, signature, public_key)
}

/// Canonical XRPL transaction hash (SHA-512Half of prefix + blob).
///
/// # Errors
///
/// Returns [`XrplCodecError::Codec`] when the blob is not valid hex.
pub fn signed_tx_hash(signed_tx_blob: &str) -> Result<String, XrplCodecError> {
    let bytes = hex::decode(signed_tx_blob).map_err(|e| XrplCodecError::Codec(e.to_string()))?;
    let mut hasher = Sha512::new();
    hasher.update(TRANSACTION_HASH_PREFIX);
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let half = digest
        .get(..32)
        .ok_or_else(|| XrplCodecError::Codec("sha512 digest shorter than 32 bytes".to_owned()))?;
    Ok(hex::encode_upper(half))
}

/// SHA-256 of `invoice_id` as a 64-character uppercase hex `InvoiceID`.
#[must_use]
pub fn invoice_id_to_field(invoice_id: &str) -> String {
    let digest = Sha256::digest(invoice_id.as_bytes());
    hex::encode_upper(digest)
}

/// Returns `true` for compressed secp256k1 (`02`/`03`) or ed25519 (`ED`) keys.
#[must_use]
pub fn is_canonical_signing_pub_key(key: &str) -> bool {
    if key.len() != CANONICAL_SIGNING_PUB_KEY_LEN {
        return false;
    }
    let bytes = key.as_bytes();
    let prefix_ok = matches!(
        (bytes.first().copied(), bytes.get(1).copied()),
        (Some(b'0'), Some(b'2' | b'3')) | (Some(b'E' | b'e'), Some(b'D' | b'd'))
    );
    prefix_ok && bytes.iter().all(u8::is_ascii_hexdigit)
}

/// Maximum `LastLedgerSequence` for `max_timeout_seconds` at `current_ledger`.
#[must_use]
pub fn max_last_ledger_sequence(current_ledger: u32, max_timeout_seconds: u64) -> u32 {
    let delta = max_timeout_seconds
        .div_ceil(DEFAULT_LEDGER_CLOSE_SECONDS)
        .saturating_add(DEFAULT_LEDGER_TOLERANCE);
    current_ledger.saturating_add(u32::try_from(delta).unwrap_or(u32::MAX))
}

/// Compares non-negative decimal strings without floating point.
///
/// # Errors
///
/// Returns [`XrplCodecError::Codec`] when either string is not a decimal.
pub fn compare_decimal_strings(left: &str, right: &str) -> Result<Ordering, XrplCodecError> {
    let left = normalize_decimal(left)?;
    let right = normalize_decimal(right)?;
    if left.whole.len() != right.whole.len() {
        return Ok(if left.whole.len() > right.whole.len() {
            Ordering::Greater
        } else {
            Ordering::Less
        });
    }
    match left.whole.cmp(&right.whole) {
        Ordering::Equal => {
            let width = left.fraction.len().max(right.fraction.len());
            let left_frac = pad_fraction(&left.fraction, width);
            let right_frac = pad_fraction(&right.fraction, width);
            Ok(left_frac.cmp(&right_frac))
        }
        other => Ok(other),
    }
}

struct NormalizedDecimal {
    /// Whole digits with leading zeros stripped.
    whole: String,
    /// Fractional digits with trailing zeros stripped.
    fraction: String,
}

fn normalize_decimal(value: &str) -> Result<NormalizedDecimal, XrplCodecError> {
    if !is_decimal_string(value) {
        return Err(XrplCodecError::Codec(format!(
            "invalid decimal string: {value}"
        )));
    }
    let (raw_whole, raw_fraction) = value.split_once('.').map_or((value, ""), |parts| parts);
    let whole = raw_whole.trim_start_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    let fraction = raw_fraction.trim_end_matches('0');
    Ok(NormalizedDecimal {
        whole: whole.to_owned(),
        fraction: fraction.to_owned(),
    })
}

fn pad_fraction(fraction: &str, width: usize) -> String {
    let mut out = String::with_capacity(width);
    out.push_str(fraction);
    while out.len() < width {
        out.push('0');
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn invoice_id_is_sha256_hex() {
        let field = invoice_id_to_field("INV-1");
        assert_eq!(field.len(), 64);
        assert!(
            field
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_lowercase())
        );
    }

    #[test]
    fn decimal_compare_is_exact() {
        assert_eq!(
            compare_decimal_strings("10.5", "10.50").unwrap(),
            Ordering::Equal
        );
        assert_eq!(
            compare_decimal_strings("10.5", "10.49").unwrap(),
            Ordering::Greater
        );
        assert_eq!(
            compare_decimal_strings("0.000001", "0.00001").unwrap(),
            Ordering::Less
        );
        assert_eq!(
            compare_decimal_strings("100", "99").unwrap(),
            Ordering::Greater
        );
    }

    #[test]
    fn canonical_pub_key_pattern() {
        assert!(is_canonical_signing_pub_key(
            "ED01FA53FA5A7E77798F882ECE20B1ABC00BB358A9E55A202D0D0676BD0CE37A63"
        ));
        assert!(is_canonical_signing_pub_key(
            "03AB40A0490F9B7ED8DF29D246BF2D6269820A0EE7742ACDD457BEA7C7D0931EDB"
        ));
        assert!(!is_canonical_signing_pub_key("01AB"));
        assert!(!is_canonical_signing_pub_key(
            "04AB40A0490F9B7ED8DF29D246BF2D6269820A0EE7742ACDD457BEA7C7D0931EDB"
        ));
    }

    #[test]
    fn last_ledger_matches_timeout_policy() {
        assert_eq!(max_last_ledger_sequence(980, 60), 994);
    }
}
