//! Canonical Algorand msgpack encoding.
//!
//! Map keys are sorted lexicographically and default values are omitted, matching
//! go-algorand's canonical codec. Addresses, hashes, and signatures are msgpack
//! `bin`; integers use the shortest unsigned encoding.

use std::collections::BTreeMap;

use data_encoding::BASE32_NOPAD;
use sha2::{Digest, Sha512_256};

use super::types::AlgorandAddress;

/// Errors from encoding or decoding Algorand transactions.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    /// Input ended before a complete msgpack value was read.
    #[error("truncated algorand msgpack")]
    Truncated,
    /// A msgpack type or field was not a valid transaction encoding.
    #[error("invalid algorand msgpack: {0}")]
    Invalid(String),
}

/// Algorand transaction type string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxnType {
    /// Native ALGO payment (`pay`).
    Pay,
    /// ASA transfer (`axfer`).
    Axfer,
    /// Any other protocol type (application call, keyreg, …).
    Other,
}

impl TxnType {
    /// Wire `type` field.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pay => "pay",
            Self::Axfer => "axfer",
            Self::Other => "unknown",
        }
    }

    fn from_wire(s: &str) -> Self {
        match s {
            "pay" => Self::Pay,
            "axfer" => Self::Axfer,
            _ => Self::Other,
        }
    }
}

/// Decoded Algorand transaction (inner `txn` object).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    /// Transaction type.
    pub txn_type: TxnType,
    /// Sender.
    pub sender: AlgorandAddress,
    /// Fee in µAlgo. Zero is omitted on the wire.
    pub fee: u64,
    /// First valid round.
    pub first_valid: u64,
    /// Last valid round.
    pub last_valid: u64,
    /// Optional note.
    pub note: Vec<u8>,
    /// Genesis ID (`gen`). Empty is omitted.
    pub genesis_id: String,
    /// Genesis hash (`gh`).
    pub genesis_hash: [u8; 32],
    /// Atomic group ID (`grp`).
    pub group: Option<[u8; 32]>,
    /// Lease (`lx`).
    pub lease: Option<[u8; 32]>,
    /// Rekey-to.
    pub rekey_to: Option<AlgorandAddress>,
    /// Payment receiver (`rcv`).
    pub receiver: Option<AlgorandAddress>,
    /// Payment amount (`amt`).
    pub amount: u64,
    /// Close remainder to (`close`).
    pub close_remainder_to: Option<AlgorandAddress>,
    /// ASA id (`xaid`).
    pub asset_id: u64,
    /// ASA amount (`aamt`).
    pub asset_amount: u64,
    /// ASA sender for clawback (`asnd`).
    pub asset_sender: Option<AlgorandAddress>,
    /// ASA receiver (`arcv`).
    pub asset_receiver: Option<AlgorandAddress>,
    /// ASA close-to (`aclose`).
    pub asset_close_to: Option<AlgorandAddress>,
}

impl Transaction {
    /// Empty transaction with zeroed hashes. Fill required fields before encoding.
    #[must_use]
    pub const fn new(txn_type: TxnType, sender: AlgorandAddress) -> Self {
        Self {
            txn_type,
            sender,
            fee: 0,
            first_valid: 0,
            last_valid: 0,
            note: Vec::new(),
            genesis_id: String::new(),
            genesis_hash: [0u8; 32],
            group: None,
            lease: None,
            rekey_to: None,
            receiver: None,
            amount: 0,
            close_remainder_to: None,
            asset_id: 0,
            asset_amount: 0,
            asset_sender: None,
            asset_receiver: None,
            asset_close_to: None,
        }
    }
}

/// Signed or unsigned transaction envelope (`{sig?, txn}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedTransaction {
    /// Ed25519 signature over `TX || msgpack(txn)`. Absent for unsigned fee-payer txns.
    pub sig: Option<[u8; 64]>,
    /// Inner transaction.
    pub txn: Transaction,
}

impl SignedTransaction {
    /// Returns `true` when a 64-byte ed25519 signature is present.
    #[must_use]
    pub const fn has_signature(&self) -> bool {
        self.sig.is_some()
    }
}

/// Encodes the inner transaction (canonical msgpack).
#[must_use]
pub fn encode_txn(txn: &Transaction) -> Vec<u8> {
    let mut fields: Vec<(&str, Vec<u8>)> = Vec::new();
    push_u64(&mut fields, "aamt", txn.asset_amount);
    push_addr(&mut fields, "aclose", txn.asset_close_to);
    push_addr(&mut fields, "arcv", txn.asset_receiver);
    push_addr(&mut fields, "asnd", txn.asset_sender);
    push_u64(&mut fields, "amt", txn.amount);
    push_addr(&mut fields, "close", txn.close_remainder_to);
    push_u64(&mut fields, "fee", txn.fee);
    push_u64(&mut fields, "fv", txn.first_valid);
    push_str(&mut fields, "gen", &txn.genesis_id);
    push_digest(&mut fields, "gh", Some(txn.genesis_hash));
    push_digest(&mut fields, "grp", txn.group);
    push_u64(&mut fields, "lv", txn.last_valid);
    push_bin(&mut fields, "note", &txn.note);
    push_digest(&mut fields, "lx", txn.lease);
    push_addr(&mut fields, "rcv", txn.receiver);
    push_addr(&mut fields, "rekey", txn.rekey_to);
    push_addr(&mut fields, "snd", Some(txn.sender));
    if txn.txn_type != TxnType::Other {
        let mut encoded = Vec::new();
        encode_str(&mut encoded, txn.txn_type.as_str());
        fields.push(("type", encoded));
    }
    push_u64(&mut fields, "xaid", txn.asset_id);
    fields.sort_by(|a, b| a.0.cmp(b.0));
    let mut out = Vec::new();
    encode_map_header(&mut out, fields.len());
    for (key, value) in fields {
        encode_str(&mut out, key);
        out.extend(value);
    }
    out
}

/// Encodes a signed-txn envelope. Unsigned fee-payer txns omit `sig`.
#[must_use]
pub fn encode_signed(stxn: &SignedTransaction) -> Vec<u8> {
    let mut fields: Vec<(&str, Vec<u8>)> = Vec::new();
    if let Some(sig) = stxn.sig {
        let mut encoded = Vec::new();
        encode_bin(&mut encoded, &sig);
        fields.push(("sig", encoded));
    }
    let mut txn_bytes = Vec::new();
    txn_bytes.extend(encode_txn(&stxn.txn));
    fields.push(("txn", txn_bytes));
    fields.sort_by(|a, b| a.0.cmp(b.0));
    let mut out = Vec::new();
    encode_map_header(&mut out, fields.len());
    for (key, value) in fields {
        encode_str(&mut out, key);
        out.extend(value);
    }
    out
}

/// Decodes a payment-group element: `{sig?, txn}` or a raw inner txn map.
///
/// # Errors
///
/// Returns [`CodecError`] when the bytes are not a transaction map.
pub fn decode_signed(bytes: &[u8]) -> Result<SignedTransaction, CodecError> {
    let value = decode_value(bytes)?;
    decode_signed_value(&value)
}

fn decode_signed_value(value: &MpValue) -> Result<SignedTransaction, CodecError> {
    let MpValue::Map(map) = value else {
        return Err(CodecError::Invalid("expected map".to_owned()));
    };
    if map.contains_key("txn") {
        let txn = map
            .get("txn")
            .ok_or_else(|| CodecError::Invalid("missing txn".to_owned()))?;
        let inner = decode_txn_map(txn)?;
        let sig = match map.get("sig") {
            Some(MpValue::Bin(b)) if b.len() == 64 => {
                let mut sig = [0u8; 64];
                sig.copy_from_slice(b);
                Some(sig)
            }
            Some(_) => return Err(CodecError::Invalid("invalid sig".to_owned())),
            None => None,
        };
        return Ok(SignedTransaction { sig, txn: inner });
    }
    if map.contains_key("type") || map.contains_key("snd") {
        let inner = decode_txn_map(value)?;
        return Ok(SignedTransaction {
            sig: None,
            txn: inner,
        });
    }
    Err(CodecError::Invalid(
        "not an algorand transaction".to_owned(),
    ))
}

fn decode_txn_map(value: &MpValue) -> Result<Transaction, CodecError> {
    let MpValue::Map(map) = value else {
        return Err(CodecError::Invalid("txn is not a map".to_owned()));
    };
    let type_str = match map.get("type") {
        Some(MpValue::Str(s)) => s.as_str(),
        None => "",
        Some(_) => return Err(CodecError::Invalid("type must be a string".to_owned())),
    };
    let mut txn = Transaction::new(TxnType::from_wire(type_str), AlgorandAddress::ZERO);
    txn.sender = addr_field(map, "snd")?.unwrap_or(AlgorandAddress::ZERO);
    txn.fee = u64_field(map, "fee")?;
    txn.first_valid = u64_field(map, "fv")?;
    txn.last_valid = u64_field(map, "lv")?;
    txn.note = bin_field(map, "note")?.unwrap_or_default();
    txn.genesis_id = str_field(map, "gen")?.unwrap_or_default();
    txn.genesis_hash = digest_field(map, "gh")?.unwrap_or([0u8; 32]);
    txn.group = digest_field(map, "grp")?;
    txn.lease = digest_field(map, "lx")?;
    txn.rekey_to = addr_field(map, "rekey")?;
    txn.receiver = addr_field(map, "rcv")?;
    txn.amount = u64_field(map, "amt")?;
    txn.close_remainder_to = addr_field(map, "close")?;
    txn.asset_id = u64_field(map, "xaid")?;
    txn.asset_amount = u64_field(map, "aamt")?;
    txn.asset_sender = addr_field(map, "asnd")?;
    txn.asset_receiver = addr_field(map, "arcv")?;
    txn.asset_close_to = addr_field(map, "aclose")?;
    Ok(txn)
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

/// Encodes a msgpack value (used to wrap signed txns in a simulate request).
pub(crate) fn encode_value(value: &MpValue) -> Vec<u8> {
    let mut out = Vec::new();
    write_value(&mut out, value);
    out
}

pub(crate) fn decode_value(bytes: &[u8]) -> Result<MpValue, CodecError> {
    let mut r = Reader {
        data: bytes,
        pos: 0,
    };
    let value = read_value(&mut r)?;
    if r.pos != bytes.len() {
        return Err(CodecError::Invalid("trailing msgpack bytes".to_owned()));
    }
    Ok(value)
}

/// Msgpack value subset used by Algorand transactions and algod simulate.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MpValue {
    U64(u64),
    Str(String),
    Bin(Vec<u8>),
    Map(BTreeMap<String, Self>),
    Array(Vec<Self>),
    Nil,
}

fn sha512_256(data: &[u8]) -> [u8; 32] {
    let digest = Sha512_256::digest(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn push_u64<'a>(fields: &mut Vec<(&'a str, Vec<u8>)>, key: &'a str, value: u64) {
    if value == 0 {
        return;
    }
    let mut encoded = Vec::new();
    encode_u64(&mut encoded, value);
    fields.push((key, encoded));
}

fn push_str<'a>(fields: &mut Vec<(&'a str, Vec<u8>)>, key: &'a str, value: &str) {
    if value.is_empty() {
        return;
    }
    let mut encoded = Vec::new();
    encode_str(&mut encoded, value);
    fields.push((key, encoded));
}

fn push_bin<'a>(fields: &mut Vec<(&'a str, Vec<u8>)>, key: &'a str, value: &[u8]) {
    if value.is_empty() {
        return;
    }
    let mut encoded = Vec::new();
    encode_bin(&mut encoded, value);
    fields.push((key, encoded));
}

fn push_digest<'a>(fields: &mut Vec<(&'a str, Vec<u8>)>, key: &'a str, value: Option<[u8; 32]>) {
    let Some(bytes) = value else {
        return;
    };
    if bytes == [0u8; 32] {
        return;
    }
    let mut encoded = Vec::new();
    encode_bin(&mut encoded, &bytes);
    fields.push((key, encoded));
}

fn push_addr<'a>(
    fields: &mut Vec<(&'a str, Vec<u8>)>,
    key: &'a str,
    value: Option<AlgorandAddress>,
) {
    let Some(addr) = value else {
        return;
    };
    if addr.is_zero() {
        return;
    }
    let mut encoded = Vec::new();
    encode_bin(&mut encoded, addr.as_bytes());
    fields.push((key, encoded));
}

#[allow(
    clippy::checked_conversions,
    reason = "msgpack length prefixes are width-selected by explicit ranges"
)]
fn encode_map_header(buf: &mut Vec<u8>, n: usize) {
    if n <= 15 {
        #[allow(clippy::cast_possible_truncation, reason = "n <= 15")]
        buf.push(0x80 | n as u8);
    } else if n <= usize::from(u16::MAX) {
        buf.push(0xde);
        #[allow(clippy::cast_possible_truncation, reason = "n <= u16::MAX")]
        buf.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        buf.push(0xdf);
        #[allow(
            clippy::cast_possible_truncation,
            reason = "map len fits u32 in practice"
        )]
        buf.extend_from_slice(&(n as u32).to_be_bytes());
    }
}

#[allow(
    clippy::checked_conversions,
    reason = "msgpack integer prefixes are width-selected by explicit ranges"
)]
fn encode_u64(buf: &mut Vec<u8>, value: u64) {
    if value <= 127 {
        #[allow(clippy::cast_possible_truncation, reason = "value <= 127")]
        buf.push(value as u8);
    } else if value <= u64::from(u8::MAX) {
        buf.push(0xcc);
        #[allow(clippy::cast_possible_truncation, reason = "value <= u8::MAX")]
        buf.push(value as u8);
    } else if value <= u64::from(u16::MAX) {
        buf.push(0xcd);
        #[allow(clippy::cast_possible_truncation, reason = "value <= u16::MAX")]
        buf.extend_from_slice(&(value as u16).to_be_bytes());
    } else if value <= u64::from(u32::MAX) {
        buf.push(0xce);
        #[allow(clippy::cast_possible_truncation, reason = "value <= u32::MAX")]
        buf.extend_from_slice(&(value as u32).to_be_bytes());
    } else {
        buf.push(0xcf);
        buf.extend_from_slice(&value.to_be_bytes());
    }
}

#[allow(
    clippy::checked_conversions,
    reason = "msgpack string prefixes are width-selected by explicit ranges"
)]
fn encode_str(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    let len = bytes.len();
    if len <= 31 {
        #[allow(clippy::cast_possible_truncation, reason = "len <= 31")]
        buf.push(0xa0 | len as u8);
    } else if len <= usize::from(u8::MAX) {
        buf.push(0xd9);
        #[allow(clippy::cast_possible_truncation, reason = "len <= u8::MAX")]
        buf.push(len as u8);
    } else if len <= usize::from(u16::MAX) {
        buf.push(0xda);
        #[allow(clippy::cast_possible_truncation, reason = "len <= u16::MAX")]
        buf.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        buf.push(0xdb);
        #[allow(clippy::cast_possible_truncation, reason = "str len fits u32")]
        buf.extend_from_slice(&(len as u32).to_be_bytes());
    }
    buf.extend_from_slice(bytes);
}

#[allow(
    clippy::checked_conversions,
    reason = "msgpack binary prefixes are width-selected by explicit ranges"
)]
fn encode_bin(buf: &mut Vec<u8>, data: &[u8]) {
    let len = data.len();
    if len <= usize::from(u8::MAX) {
        buf.push(0xc4);
        #[allow(clippy::cast_possible_truncation, reason = "len <= u8::MAX")]
        buf.push(len as u8);
    } else if len <= usize::from(u16::MAX) {
        buf.push(0xc5);
        #[allow(clippy::cast_possible_truncation, reason = "len <= u16::MAX")]
        buf.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        buf.push(0xc6);
        #[allow(clippy::cast_possible_truncation, reason = "bin len fits u32")]
        buf.extend_from_slice(&(len as u32).to_be_bytes());
    }
    buf.extend_from_slice(data);
}

fn write_value(buf: &mut Vec<u8>, value: &MpValue) {
    match value {
        MpValue::Nil => buf.push(0xc0),
        MpValue::U64(v) => encode_u64(buf, *v),
        MpValue::Str(s) => encode_str(buf, s),
        MpValue::Bin(b) => encode_bin(buf, b),
        MpValue::Array(items) => {
            encode_array_header(buf, items.len());
            for item in items {
                write_value(buf, item);
            }
        }
        MpValue::Map(map) => {
            encode_map_header(buf, map.len());
            for (k, v) in map {
                encode_str(buf, k);
                write_value(buf, v);
            }
        }
    }
}

#[allow(
    clippy::checked_conversions,
    reason = "msgpack array prefixes are width-selected by explicit ranges"
)]
fn encode_array_header(buf: &mut Vec<u8>, n: usize) {
    if n <= 15 {
        #[allow(clippy::cast_possible_truncation, reason = "n <= 15")]
        buf.push(0x90 | n as u8);
    } else if n <= usize::from(u16::MAX) {
        buf.push(0xdc);
        #[allow(clippy::cast_possible_truncation, reason = "n <= u16::MAX")]
        buf.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        buf.push(0xdd);
        #[allow(clippy::cast_possible_truncation, reason = "array len fits u32")]
        buf.extend_from_slice(&(n as u32).to_be_bytes());
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], CodecError> {
        let end = self.pos.checked_add(n).ok_or(CodecError::Truncated)?;
        let slice = self.data.get(self.pos..end).ok_or(CodecError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, CodecError> {
        let b = self.take(1)?;
        b.first().copied().ok_or(CodecError::Truncated)
    }
}

fn read_value(r: &mut Reader<'_>) -> Result<MpValue, CodecError> {
    let tag = r.u8()?;
    match tag {
        0xc0 => Ok(MpValue::Nil),
        0x00..=0x7f => Ok(MpValue::U64(u64::from(tag))),
        0xcc => Ok(MpValue::U64(u64::from(r.u8()?))),
        0xcd => {
            let b = r.take(2)?;
            let arr: [u8; 2] = b
                .try_into()
                .map_err(|_| CodecError::Invalid("u16".to_owned()))?;
            Ok(MpValue::U64(u64::from(u16::from_be_bytes(arr))))
        }
        0xce => {
            let b = r.take(4)?;
            let arr: [u8; 4] = b
                .try_into()
                .map_err(|_| CodecError::Invalid("u32".to_owned()))?;
            Ok(MpValue::U64(u64::from(u32::from_be_bytes(arr))))
        }
        0xcf => {
            let b = r.take(8)?;
            let arr: [u8; 8] = b
                .try_into()
                .map_err(|_| CodecError::Invalid("u64".to_owned()))?;
            Ok(MpValue::U64(u64::from_be_bytes(arr)))
        }
        0xc4 => {
            let len = usize::from(r.u8()?);
            Ok(MpValue::Bin(r.take(len)?.to_vec()))
        }
        0xc5 => {
            let b = r.take(2)?;
            let arr: [u8; 2] = b
                .try_into()
                .map_err(|_| CodecError::Invalid("bin16".to_owned()))?;
            let len = usize::from(u16::from_be_bytes(arr));
            Ok(MpValue::Bin(r.take(len)?.to_vec()))
        }
        0xa0..=0xbf => {
            let len = usize::from(tag & 0x1f);
            let bytes = r.take(len)?;
            Ok(str_or_bin(bytes))
        }
        0xd9 => {
            let len = usize::from(r.u8()?);
            let bytes = r.take(len)?;
            Ok(str_or_bin(bytes))
        }
        0x80..=0x8f => read_map(r, usize::from(tag & 0x0f)),
        0xde => {
            let b = r.take(2)?;
            let arr: [u8; 2] = b
                .try_into()
                .map_err(|_| CodecError::Invalid("map16".to_owned()))?;
            read_map(r, usize::from(u16::from_be_bytes(arr)))
        }
        0x90..=0x9f => read_array(r, usize::from(tag & 0x0f)),
        0xdc => {
            let b = r.take(2)?;
            let arr: [u8; 2] = b
                .try_into()
                .map_err(|_| CodecError::Invalid("array16".to_owned()))?;
            read_array(r, usize::from(u16::from_be_bytes(arr)))
        }
        other => Err(CodecError::Invalid(format!(
            "unsupported msgpack tag {other:#x}"
        ))),
    }
}

fn str_or_bin(bytes: &[u8]) -> MpValue {
    std::str::from_utf8(bytes).map_or_else(
        |_| MpValue::Bin(bytes.to_vec()),
        |s| MpValue::Str(s.to_owned()),
    )
}

fn read_map(r: &mut Reader<'_>, n: usize) -> Result<MpValue, CodecError> {
    let mut map = BTreeMap::new();
    for _ in 0..n {
        let key = match read_value(r)? {
            MpValue::Str(s) => s,
            MpValue::Bin(b) => String::from_utf8(b)
                .map_err(|_| CodecError::Invalid("map key not utf-8".to_owned()))?,
            _ => return Err(CodecError::Invalid("map key must be a string".to_owned())),
        };
        let value = read_value(r)?;
        let _ = map.insert(key, value);
    }
    Ok(MpValue::Map(map))
}

fn read_array(r: &mut Reader<'_>, n: usize) -> Result<MpValue, CodecError> {
    let mut items = Vec::with_capacity(n);
    for _ in 0..n {
        items.push(read_value(r)?);
    }
    Ok(MpValue::Array(items))
}

fn u64_field(map: &BTreeMap<String, MpValue>, key: &str) -> Result<u64, CodecError> {
    match map.get(key) {
        None => Ok(0),
        Some(MpValue::U64(v)) => Ok(*v),
        Some(_) => Err(CodecError::Invalid(format!("{key} must be uint"))),
    }
}

fn str_field(map: &BTreeMap<String, MpValue>, key: &str) -> Result<Option<String>, CodecError> {
    match map.get(key) {
        None => Ok(None),
        Some(MpValue::Str(s)) => Ok(Some(s.clone())),
        Some(_) => Err(CodecError::Invalid(format!("{key} must be string"))),
    }
}

fn bin_field(map: &BTreeMap<String, MpValue>, key: &str) -> Result<Option<Vec<u8>>, CodecError> {
    match map.get(key) {
        None => Ok(None),
        Some(MpValue::Bin(b)) => Ok(Some(b.clone())),
        Some(MpValue::Str(s)) => Ok(Some(s.as_bytes().to_vec())),
        Some(_) => Err(CodecError::Invalid(format!("{key} must be binary"))),
    }
}

fn digest_field(
    map: &BTreeMap<String, MpValue>,
    key: &str,
) -> Result<Option<[u8; 32]>, CodecError> {
    match bin_field(map, key)? {
        None => Ok(None),
        Some(b) if b.len() == 32 => {
            let mut out = [0u8; 32];
            out.copy_from_slice(&b);
            Ok(Some(out))
        }
        Some(_) => Err(CodecError::Invalid(format!("{key} must be 32 bytes"))),
    }
}

fn addr_field(
    map: &BTreeMap<String, MpValue>,
    key: &str,
) -> Result<Option<AlgorandAddress>, CodecError> {
    match bin_field(map, key)? {
        None => Ok(None),
        Some(b) if b.len() == 32 => {
            let mut out = [0u8; 32];
            out.copy_from_slice(&b);
            Ok(Some(AlgorandAddress::from_public_key(out)))
        }
        Some(_) => Err(CodecError::Invalid(format!("{key} must be 32 bytes"))),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use base64::Engine;

    use super::*;

    const SPEC_PAY: &str = "gaN0eG6Jo2ZlZc0H0KJmds4DLgNro2dlbqxtYWlubmV0LXYxLjCiZ2jEIMBhxNj8Hb3e0tdgS+RWjj9tBBmHrDe95LYgtas5JIrfo2dycMQgfy1Szr+lgvgTJsviMY2KnHSsXqyfCJ1UOCE+2Tf3vS+ibHbOAy4HU6NyY3bEICgEhaJgm6IBjiSUgAAAAAAAAAAAAAAAAAAAAAAAAAAAo3NuZMQgKASFomCbogGOJJSAAAAAAAAAAAAAAAAAAAAAAAAAAACkdHlwZaNwYXk=";
    const SPEC_AXFER: &str = "gqNzaWfEQP3J1DI6GLSfK0nLZftvSyVMJuFOE48xPlnZpNdEJWbGbcxsD5aASwza4TjbwhgEF0dXOv8E3W/f22vkEzfFywWjdHhuiaRhYW10zgBMS0CkYXJjdsQgiSTqRESRI1JEAxxJKQAAAAAAAAAAAAAAAAAAAAAAAACiZnbOAy4Da6JnaMQgwGHE2Pwdvd7S12BL5FaOP20EGYesN73ktiC1qzkkit+jZ3JwxCB/LVLOv6WC+BMmy+IxjYqcdKxerJ8InVQ4IT7ZN/e9L6Jsds4DLgdTo3NuZMQgEtBGzAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACkdHlwZaVheGZlcqR4YWlkzgHhq3A=";

    #[test]
    fn spec_pay_roundtrip() {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(SPEC_PAY)
            .unwrap();
        let decoded = decode_signed(&raw).unwrap();
        assert_eq!(decoded.txn.txn_type, TxnType::Pay);
        assert_eq!(decoded.txn.fee, 2000);
        assert_eq!(decoded.txn.amount, 0);
        assert_eq!(decoded.txn.genesis_id, "mainnet-v1.0");
        assert!(decoded.sig.is_none());
        assert_eq!(encode_signed(&decoded), raw);
    }

    #[test]
    fn spec_axfer_roundtrip() {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(SPEC_AXFER)
            .unwrap();
        let decoded = decode_signed(&raw).unwrap();
        assert_eq!(decoded.txn.txn_type, TxnType::Axfer);
        assert_eq!(decoded.txn.asset_amount, 5_000_000);
        assert_eq!(decoded.txn.asset_id, 31_566_704);
        assert_eq!(decoded.txn.fee, 0);
        assert!(decoded.has_signature());
        assert_eq!(encode_signed(&decoded), raw);
    }

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
}
