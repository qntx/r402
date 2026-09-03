//! Atomic group ID, transaction ID, and fee helpers.

use data_encoding::BASE32_NOPAD;
use sha2::{Digest, Sha512_256};

use super::{
    Transaction, encode_array_header, encode_bin, encode_map_header, encode_str, encode_txn,
};

/// Maximum number of top-level transactions in an Algorand atomic group.
pub const MAX_TRANSACTION_GROUP_SIZE: usize = 16;

/// Per-transaction fee cap used to bound facilitator fee-payer spend (µAlgo).
pub const MAX_REASONABLE_FEE_PER_TXN: u64 = 5_000;

/// Maximum acceptable fee-payer fee for a group of `group_size` transactions.
#[must_use]
pub fn max_reasonable_group_fee(group_size: usize) -> u64 {
    let size = u64::try_from(group_size).unwrap_or(u64::MAX);
    MAX_REASONABLE_FEE_PER_TXN.saturating_mul(size)
}

/// Computes the atomic group ID.
///
/// `SHA512_256("TG" || msgpack({txlist: [txid(txn without grp), …]}))`
/// where `txid` is `SHA512_256("TX" || msgpack(txn))`.
#[must_use]
pub fn compute_group_id(txns: &[Transaction]) -> [u8; 32] {
    let mut hashes = Vec::with_capacity(txns.len());
    for txn in txns {
        let mut stripped = txn.clone();
        stripped.group = None;
        hashes.push(txid(&stripped));
    }
    let mut txlist = Vec::new();
    encode_array_header(&mut txlist, hashes.len());
    for hash in &hashes {
        encode_bin(&mut txlist, hash);
    }
    let mut group_obj = Vec::new();
    encode_map_header(&mut group_obj, 1);
    encode_str(&mut group_obj, "txlist");
    group_obj.extend(txlist);
    let mut buf = Vec::from(*b"TG");
    buf.extend(group_obj);
    sha512_256(&buf)
}

/// Assigns the same group ID to every transaction and returns the updated set.
#[must_use]
pub fn assign_group(mut txns: Vec<Transaction>) -> Vec<Transaction> {
    if txns.len() <= 1 {
        if let Some(txn) = txns.first_mut() {
            txn.group = None;
        }
        return txns;
    }
    let group = compute_group_id(&txns);
    for txn in &mut txns {
        txn.group = Some(group);
    }
    txns
}

/// Transaction ID bytes: `SHA512_256("TX" || msgpack(txn))`.
#[must_use]
pub fn txid(txn: &Transaction) -> [u8; 32] {
    let mut buf = Vec::from(*b"TX");
    buf.extend(encode_txn(txn));
    sha512_256(&buf)
}

/// 52-character base32 transaction ID (no padding).
#[must_use]
pub fn txid_str(txn: &Transaction) -> String {
    BASE32_NOPAD.encode(&txid(txn))
}

/// Bytes signed by ed25519: `TX || msgpack(txn)`.
#[must_use]
pub fn bytes_for_signing(txn: &Transaction) -> Vec<u8> {
    let mut buf = Vec::from(*b"TX");
    buf.extend(encode_txn(txn));
    buf
}

/// Protocol fee for a transaction of `size` bytes.
#[must_use]
pub fn txn_fee(fee_per_byte: u64, min_fee: u64, size: usize) -> u64 {
    let size = u64::try_from(size).unwrap_or(u64::MAX);
    fee_per_byte.saturating_mul(size).max(min_fee)
}

fn sha512_256(data: &[u8]) -> [u8; 32] {
    let digest = Sha512_256::digest(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

#[cfg(test)]
mod tests {
    use base64::Engine;

    use super::*;
    use crate::chain::codec::{SPEC_AXFER, SPEC_PAY, Transaction, decode_signed};

    #[test]
    fn group_id_is_stable() {
        let raw_pay = base64::engine::general_purpose::STANDARD
            .decode(SPEC_PAY)
            .unwrap();
        let raw_axfer = base64::engine::general_purpose::STANDARD
            .decode(SPEC_AXFER)
            .unwrap();
        let pay = decode_signed(&raw_pay).unwrap().txn;
        let axfer = decode_signed(&raw_axfer).unwrap().txn;
        assert_eq!(pay.group, axfer.group);
        let regrouped = assign_group(vec![
            Transaction {
                group: None,
                ..pay.clone()
            },
            Transaction {
                group: None,
                ..axfer.clone()
            },
        ]);
        assert_eq!(regrouped.first().and_then(|t| t.group), pay.group);
        assert_eq!(regrouped.get(1).and_then(|t| t.group), axfer.group);
    }

    #[test]
    fn encode_size_used_for_fee() {
        let txn = Transaction::new(
            super::super::TxnType::Pay,
            crate::chain::AlgorandAddress::from_public_key([1u8; 32]),
        );
        let size = encode_txn(&txn).len();
        assert!(size > 0, "canonical encoding is non-empty");
        assert_eq!(txn_fee(0, 1000, size), 1000);
        assert_eq!(txn_fee(10, 1000, 50), 1000);
    }
}
