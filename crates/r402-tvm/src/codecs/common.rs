//! Shared BoC and address helpers.

use base64::Engine;
use tonlib_core::TonAddress;
use tonlib_core::cell::{BagOfCells, Cell, CellBuilder, TonCellError};
use tonlib_core::types::TonHash;

use crate::chain::{TvmAddress, TvmAddressFormatError, TvmChainReference};

/// Errors from BoC encode/decode.
#[derive(Debug, thiserror::Error)]
pub enum BocError {
    /// Cell build or parse failed.
    #[error("{0}")]
    Cell(#[from] TonCellError),
    /// Expected a single-root BoC.
    #[error("{0}")]
    Invalid(String),
}

/// Normalizes a TON address to raw `workchain:hex`.
///
/// # Errors
///
/// Returns [`TvmAddressFormatError`] if `address` is not a TON address.
pub fn normalize_address(address: &str) -> Result<TvmAddress, TvmAddressFormatError> {
    address.parse()
}

/// Encodes `address` as a Toncenter `slice` stack item (address cell BoC).
///
/// # Errors
///
/// Returns [`BocError`] if the address cannot be stored in a cell.
pub fn address_to_stack_item(address: &TonAddress) -> Result<serde_json::Value, BocError> {
    let cell = CellBuilder::new().store_address(address)?.build()?;
    Ok(serde_json::json!({
        "type": "slice",
        "value": encode_base64_boc(&cell)?,
    }))
}

/// Decodes a single-root base64 BoC.
///
/// # Errors
///
/// Returns [`BocError`] if the string is empty, not base64, or not a single cell.
pub fn decode_base64_boc(value: &str) -> Result<Cell, BocError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(BocError::Invalid(
            "invalid_exact_tvm_payload_settlement_boc".to_owned(),
        ));
    }
    let boc = BagOfCells::parse_base64(trimmed)?;
    let root = boc
        .single_root()
        .map_err(|e| BocError::Invalid(e.to_string()))?;
    Ok((*root).clone())
}

/// Encodes a cell as a standard-base64 BoC with CRC32.
///
/// # Errors
///
/// Returns [`BocError`] if serialization fails.
pub fn encode_base64_boc(cell: &Cell) -> Result<String, BocError> {
    let bytes = BagOfCells::from_root(cell.clone()).serialize(true)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// A cell containing a single zero bit (TEP-74 default forward payload).
///
/// # Errors
///
/// Returns [`BocError`] if the cell cannot be built.
pub fn make_zero_bit_cell() -> Result<Cell, BocError> {
    Ok(CellBuilder::new().store_bit(false)?.build()?)
}

/// Standard-base64 (padded) encoding of a cell hash, matching Toncenter traces.
#[must_use]
pub fn cell_hash_base64(cell: &Cell) -> String {
    cell_hash_base64_from_ton_hash(&cell.cell_hash())
}

/// Standard-base64 (padded) encoding of a [`TonHash`].
#[must_use]
pub fn cell_hash_base64_from_ton_hash(hash: &TonHash) -> String {
    base64::engine::general_purpose::STANDARD.encode(hash.as_slice())
}

/// Lowercase hex encoding of a cell hash.
#[must_use]
pub fn cell_hash_hex(cell: &Cell) -> String {
    cell.cell_hash().to_hex()
}

/// Lowercase hex encoding of a [`TonHash`].
#[must_use]
pub fn ton_hash_hex(hash: &TonHash) -> String {
    hash.to_hex()
}

/// Global id for a CAIP-2 TVM network string.
///
/// # Errors
///
/// Returns [`TvmAddressFormatError`] if `network` is not `tvm:-239` / `tvm:-3`.
pub fn network_global_id(network: &str) -> Result<i32, TvmAddressFormatError> {
    let chain: TvmChainReference = network
        .parse::<r402_core::chain::ChainId>()
        .ok()
        .and_then(|id| TvmChainReference::try_from(id).ok())
        .ok_or_else(|| {
            TvmAddressFormatError::Invalid(format!("unsupported tvm network: {network}"))
        })?;
    Ok(chain.global_id())
}

/// Parses a hex or base64 BoC blob.
pub(crate) fn decode_boc_text(value: &str) -> Result<Vec<u8>, BocError> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(BocError::Invalid("BOC value is empty".to_owned()));
    }
    if normalized.len().is_multiple_of(2) && normalized.bytes().all(|b| b.is_ascii_hexdigit()) {
        let mut out = vec![0u8; normalized.len() / 2];
        for (i, chunk) in normalized.as_bytes().as_chunks::<2>().0.iter().enumerate() {
            let hi = chunk
                .first()
                .copied()
                .and_then(hex_digit)
                .ok_or_else(|| BocError::Invalid("invalid hex".to_owned()))?;
            let lo = chunk
                .get(1)
                .copied()
                .and_then(hex_digit)
                .ok_or_else(|| BocError::Invalid("invalid hex".to_owned()))?;
            if let Some(slot) = out.get_mut(i) {
                *slot = (hi << 4) | lo;
            }
        }
        return Ok(out);
    }
    base64::engine::general_purpose::STANDARD
        .decode(normalized)
        .map_err(|e| BocError::Invalid(e.to_string()))
}

const fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Loads W5R1 or Highload V3 code from a hex BoC constant.
///
/// # Errors
///
/// Returns [`BocError`] if the hex is not a single-root BoC.
pub fn code_from_hex(hex: &str) -> Result<Cell, BocError> {
    let boc = BagOfCells::parse_hex(hex)?;
    let root = boc
        .single_root()
        .map_err(|e| BocError::Invalid(e.to_string()))?;
    Ok((*root).clone())
}
