//! Highload V3 query_id, data cell, and processed-id bitmap.

use std::collections::HashMap;

use num_bigint::BigUint;
use num_traits::ToPrimitive;
use tonlib_core::cell::dict::predefined_readers::val_reader_ref_cell;
use tonlib_core::cell::{ArcCell, Cell, CellBuilder, TonCellError};

use super::common::{BocError, cell_hash_hex, code_from_hex};
use super::w5::StateInitCells;
use crate::{
    DEFAULT_HIGHLOAD_SUBWALLET_ID, DEFAULT_HIGHLOAD_TIMEOUT, HIGHLOAD_V3_CODE_HASH,
    HIGHLOAD_V3_CODE_HEX,
};

/// Maximum Highload V3 shift.
pub const MAX_SHIFT: u32 = 8191;
/// Maximum Highload V3 bit number (inclusive bound used by the bitmap).
pub const MAX_BIT_NUMBER: u32 = 1022;
/// Last usable seqno: `MAX_SHIFT * 1023 + (MAX_BIT_NUMBER - 1)`.
pub const MAX_USABLE_QUERY_SEQNO: u32 = MAX_SHIFT * 1023 + (MAX_BIT_NUMBER - 1);

/// On-chain Highload V3 query bitmaps.
#[derive(Debug, Clone, Default)]
pub struct HighloadQueryState {
    /// Previous window (`HashmapE 13 ^Cell`).
    pub old_queries: HashMap<u16, ArcCell>,
    /// Current window (`HashmapE 13 ^Cell`).
    pub queries: HashMap<u16, ArcCell>,
}

/// Converts a monotonic seqno into a Highload V3 `query_id`.
///
/// # Errors
///
/// Returns [`BocError`] if `seqno` exceeds [`MAX_USABLE_QUERY_SEQNO`].
pub fn seqno_to_query_id(seqno: u32) -> Result<u32, BocError> {
    if seqno > MAX_USABLE_QUERY_SEQNO {
        return Err(BocError::Invalid(
            "Highload V3 seqno is out of range".to_owned(),
        ));
    }
    let shift = seqno / 1023;
    let bit_number = seqno % 1023;
    Ok((shift << 10) + bit_number)
}

/// Builds the Highload V3 inner `internal_transfer` body.
///
/// # Errors
///
/// Returns [`BocError`] if the cell cannot be built.
pub fn serialize_internal_transfer(actions: &Cell, query_id: u32) -> Result<Cell, BocError> {
    Ok(CellBuilder::new()
        .store_u32(32, 0xae42_e5a4)?
        .store_u64(64, u64::from(query_id))?
        .store_reference(&actions.clone().to_arc())?
        .build()?)
}

/// Builds Highload V3 `StateInit` for `public_key`.
///
/// # Errors
///
/// Returns [`BocError`] if the code hash is unexpected or the cell cannot be built.
pub fn build_highload_v3_state_init(
    public_key: &[u8; 32],
    subwallet_id: u32,
    timeout: u32,
) -> Result<StateInitCells, BocError> {
    let code = code_from_hex(HIGHLOAD_V3_CODE_HEX)?;
    if cell_hash_hex(&code) != HIGHLOAD_V3_CODE_HASH {
        return Err(BocError::Invalid(
            "Unexpected highload-wallet-contract-v3 code hash".to_owned(),
        ));
    }
    let data = CellBuilder::new()
        .store_slice(public_key)?
        .store_u32(32, subwallet_id)?
        .store_u64(64, 0)?
        .store_u8(2, 0)?
        .store_u32(22, timeout)?
        .build()?;
    Ok(StateInitCells { code, data })
}

/// Default Highload V3 init (subwallet `0x10ad`, timeout 3600).
///
/// # Errors
///
/// Returns [`BocError`] if construction fails.
pub fn build_default_highload_v3_state_init(
    public_key: &[u8; 32],
) -> Result<StateInitCells, BocError> {
    build_highload_v3_state_init(
        public_key,
        DEFAULT_HIGHLOAD_SUBWALLET_ID,
        DEFAULT_HIGHLOAD_TIMEOUT,
    )
}

/// Loads processed query bitmaps from an active Highload V3 data cell.
///
/// # Errors
///
/// Returns [`BocError`] if the data cell is not Highload V3 layout.
pub fn load_highload_query_state(data: &Cell, now: u64) -> Result<HighloadQueryState, BocError> {
    let mut parser = data.parser();
    let _pubkey = parser.load_bytes(32)?;
    let _subwallet = parser.load_u32(32)?;
    // Contract stores HashmapE 13 ^Cell (`udict_set_ref`). Inline
    // `val_reader_cell` wraps the leaf and makes every bitmap look empty.
    let mut old_queries = parser.load_dict(13, key_reader_u13, val_reader_ref_cell)?;
    let mut queries = parser.load_dict(13, key_reader_u13, val_reader_ref_cell)?;
    let last_clean_time = parser.load_u64(64)?;
    let timeout = u64::from(parser.load_u32(22)?);

    if last_clean_time < now.saturating_sub(timeout) {
        old_queries = queries;
        queries = HashMap::new();
    }
    if last_clean_time < now.saturating_sub(timeout.saturating_mul(2)) {
        old_queries = HashMap::new();
    }

    Ok(HighloadQueryState {
        old_queries,
        queries,
    })
}

/// Returns `true` if `query_id` is already set in on-chain bitmaps.
#[must_use]
pub fn query_id_is_processed(state: &HighloadQueryState, query_id: u32) -> bool {
    let shift = (query_id >> 10) as u16;
    let bit_number = query_id & 1023;
    bitmap_contains(state.old_queries.get(&shift), bit_number)
        || bitmap_contains(state.queries.get(&shift), bit_number)
}

fn key_reader_u13(raw_key: &BigUint) -> Result<u16, TonCellError> {
    raw_key.to_u16().ok_or_else(|| {
        TonCellError::InvalidInput(format!("Highload dict key does not fit u16: {raw_key}"))
    })
}

fn bitmap_contains(bitmap: Option<&ArcCell>, bit_number: u32) -> bool {
    let Some(bitmap) = bitmap else {
        return false;
    };
    if (bit_number as usize) >= bitmap.bit_len() {
        return false;
    }
    let mut parser = bitmap.parser();
    if parser.seek(i64::from(bit_number)).is_err() {
        return false;
    }
    parser.load_bit().unwrap_or(false)
}

/// Highload V3 external body: `sign(inner.hash) || ref(inner)`.
///
/// Inner layout: `subwalletId(32) || ref(actionsMsg) || 1(8) || queryId(23)
/// || createdAt(64) || timeout(22)`.
///
/// # Errors
///
/// Returns [`BocError`] if the cell cannot be built.
/// Builds the Highload V3 signed-inner cell (the Ed25519 preimage).
///
/// # Errors
///
/// Returns [`BocError`] if the cell cannot be built.
pub fn build_highload_inner(
    subwallet_id: u32,
    actions_msg: &Cell,
    query_id: u32,
    created_at: u64,
    timeout: u32,
) -> Result<Cell, BocError> {
    Ok(CellBuilder::new()
        .store_u32(32, subwallet_id)?
        .store_reference(&actions_msg.clone().to_arc())?
        .store_u8(8, 1)?
        .store_u32(23, query_id)?
        .store_u64(64, created_at)?
        .store_u32(22, timeout)?
        .build()?)
}

/// Highload V3 external body: `sign(inner.hash) || ref(inner)`.
///
/// # Errors
///
/// Returns [`BocError`] if the cell cannot be built.
pub fn pack_highload_external_body(signature: &[u8; 64], inner: &Cell) -> Result<Cell, BocError> {
    Ok(CellBuilder::new()
        .store_slice(signature)?
        .store_reference(&inner.clone().to_arc())?
        .build()?)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn query_id_formula() {
        assert_eq!(seqno_to_query_id(0).unwrap(), 0);
        assert_eq!(seqno_to_query_id(1023).unwrap(), 1 << 10);
        assert_eq!(seqno_to_query_id(1024).unwrap(), (1 << 10) + 1);
        assert!(seqno_to_query_id(MAX_USABLE_QUERY_SEQNO).is_ok());
        assert!(seqno_to_query_id(MAX_USABLE_QUERY_SEQNO + 1).is_err());
    }

    #[test]
    fn highload_code_hash_matches_constant() {
        let code = code_from_hex(HIGHLOAD_V3_CODE_HEX).unwrap();
        assert_eq!(cell_hash_hex(&code), HIGHLOAD_V3_CODE_HASH);
    }

    #[test]
    fn query_id_is_processed_reads_ref_cell_bitmaps() {
        use std::collections::HashMap;

        use tonlib_core::cell::dict::predefined_writers::val_writer_ref_cell;

        let query_id = seqno_to_query_id(5).unwrap();
        let shift = (query_id >> 10) as u16;
        let bit_number = query_id & 1023;
        assert_eq!(shift, 0);
        assert_eq!(bit_number, 5);

        let mut bitmap = CellBuilder::new();
        for i in 0..1023u32 {
            bitmap.store_bit(i == bit_number).unwrap();
        }
        let bitmap = bitmap.build().unwrap().to_arc();

        let mut queries = HashMap::new();
        queries.insert(shift, bitmap);

        let mut data = CellBuilder::new();
        data.store_slice(&[0u8; 32]).unwrap();
        data.store_u32(32, DEFAULT_HIGHLOAD_SUBWALLET_ID).unwrap();
        data.store_bit(false).unwrap();
        data.store_dict(13, val_writer_ref_cell, queries).unwrap();
        data.store_u64(64, 1_700_000_000).unwrap();
        data.store_u32(22, DEFAULT_HIGHLOAD_TIMEOUT).unwrap();
        let data = data.build().unwrap();

        let state = load_highload_query_state(&data, 1_700_000_000).unwrap();
        assert!(
            query_id_is_processed(&state, query_id),
            "set bit in ^Cell bitmap must be visible"
        );
        assert!(
            !query_id_is_processed(&state, seqno_to_query_id(6).unwrap()),
            "unset bit must stay free"
        );
    }
}
