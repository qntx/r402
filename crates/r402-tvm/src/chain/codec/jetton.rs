//! TEP-74 `jetton_transfer` body codec.

use num_bigint::BigUint;
use tonlib_core::TonAddress;
use tonlib_core::cell::{Cell, CellBuilder};

use super::cell::{BocError, decode_base64_boc, make_zero_bit_cell};
use crate::JETTON_TRANSFER_OP;
use crate::chain::{TvmAddress, TvmAddressFormatError};
use crate::exact::error::ERR_EXACT_TVM_INVALID_JETTON_TRANSFER;
use crate::exact::payload::TvmExtra;

/// Parsed TEP-74 jetton transfer.
#[derive(Debug, Clone)]
pub struct ParsedJettonTransfer {
    /// Source jetton wallet (destination of the W5 out message).
    pub source_wallet: TvmAddress,
    /// Jetton `destination` (payTo).
    pub destination: TvmAddress,
    /// `response_destination`, or `None` for `addr_none`.
    pub response_destination: Option<TvmAddress>,
    /// Jetton amount in atomic units.
    pub jetton_amount: u128,
    /// TON attached to the W5 out message (filled by the W5 parser).
    pub attached_ton_amount: u128,
    /// TEP-74 `forward_ton_amount`.
    pub forward_ton_amount: u128,
    /// TEP-74 `forward_payload` cell.
    pub forward_payload: Cell,
    /// Hash of the jetton-transfer body cell.
    pub body_hash: tonlib_core::types::TonHash,
}

/// Builds a TEP-74 `jetton_transfer` body from payment requirements.
///
/// # Errors
///
/// Returns [`BocError`] if addresses or the optional forward payload BoC are invalid.
pub fn build_jetton_transfer_body(
    amount: u128,
    pay_to: &TvmAddress,
    extra: &TvmExtra,
) -> Result<Cell, BocError> {
    let forward_ton = extra
        .forward_ton_amount_u128()
        .map_err(|e| BocError::Invalid(format!("{e}")))?;
    let pay_to_ton = pay_to
        .to_ton()
        .map_err(|e| BocError::Invalid(e.to_string()))?;
    let response = match extra.response_destination.as_ref() {
        Some(addr) => addr
            .to_ton()
            .map_err(|e| BocError::Invalid(e.to_string()))?,
        None => TonAddress::NULL,
    };

    let mut builder = CellBuilder::new();
    builder.store_u32(32, JETTON_TRANSFER_OP)?;
    builder.store_u64(64, 0)?;
    builder.store_coins(&BigUint::from(amount))?;
    builder.store_address(&pay_to_ton)?;
    builder.store_address(&response)?;
    builder.store_bit(false)?;
    builder.store_coins(&BigUint::from(forward_ton))?;

    if let Some(payload_b64) = extra.forward_payload.as_deref() {
        let payload = decode_base64_boc(payload_b64)?;
        builder.store_bit(true)?;
        builder.store_child(payload)?;
    } else {
        builder.store_bit(false)?;
        builder.store_cell(&make_zero_bit_cell()?)?;
    }

    Ok(builder.build()?)
}

/// Parses a TEP-74 `jetton_transfer` body.
///
/// # Errors
///
/// Returns [`BocError`] if the body is not a single custom-payload-free transfer.
pub fn parse_jetton_transfer(
    jetton_wallet: &TvmAddress,
    body: &Cell,
) -> Result<ParsedJettonTransfer, BocError> {
    let mut slice = body.parser();
    if slice.remaining_bits() < 32 {
        return Err(BocError::Invalid(
            ERR_EXACT_TVM_INVALID_JETTON_TRANSFER.to_owned(),
        ));
    }
    let opcode = slice.load_u32(32)?;
    if opcode != JETTON_TRANSFER_OP {
        return Err(BocError::Invalid(
            ERR_EXACT_TVM_INVALID_JETTON_TRANSFER.to_owned(),
        ));
    }
    let _query_id = slice.load_u64(64)?;
    let amount = coins_to_u128(&slice.load_coins()?)?;
    let dest_ton = slice.load_address()?;
    if dest_ton == TonAddress::NULL {
        return Err(BocError::Invalid(
            ERR_EXACT_TVM_INVALID_JETTON_TRANSFER.to_owned(),
        ));
    }
    let destination = address_from_ton(&dest_ton)?;
    let response_ton = slice.load_address()?;
    let response_destination = if response_ton == TonAddress::NULL {
        None
    } else {
        Some(address_from_ton(&response_ton)?)
    };
    let has_custom_payload = slice.load_bit()?;
    if has_custom_payload {
        return Err(BocError::Invalid(
            ERR_EXACT_TVM_INVALID_JETTON_TRANSFER.to_owned(),
        ));
    }
    let forward_ton_amount = coins_to_u128(&slice.load_coins()?)?;
    let forward_payload = if slice.load_bit()? {
        (*slice.next_reference()?).clone()
    } else {
        remaining_as_cell(&mut slice)?
    };

    Ok(ParsedJettonTransfer {
        source_wallet: jetton_wallet.clone(),
        destination,
        response_destination,
        jetton_amount: amount,
        attached_ton_amount: 0,
        forward_ton_amount,
        forward_payload,
        body_hash: body.cell_hash(),
    })
}

fn remaining_as_cell(parser: &mut tonlib_core::cell::CellParser<'_>) -> Result<Cell, BocError> {
    let remaining_bits = parser.remaining_bits();
    let data = parser.load_bits(remaining_bits)?;
    let remaining_refs = parser.remaining_refs();
    let mut references = Vec::with_capacity(remaining_refs);
    for _ in 0..remaining_refs {
        references.push(parser.next_reference()?);
    }
    Ok(Cell::new(data, remaining_bits, references, false)?)
}

pub(crate) fn coins_to_u128(amount: &BigUint) -> Result<u128, BocError> {
    let bytes = amount.to_bytes_be();
    if bytes.len() > 16 {
        return Err(BocError::Invalid(
            ERR_EXACT_TVM_INVALID_JETTON_TRANSFER.to_owned(),
        ));
    }
    let mut padded = [0u8; 16];
    let start = 16 - bytes.len();
    let dest = padded
        .get_mut(start..)
        .ok_or_else(|| BocError::Invalid(ERR_EXACT_TVM_INVALID_JETTON_TRANSFER.to_owned()))?;
    dest.copy_from_slice(&bytes);
    Ok(u128::from_be_bytes(padded))
}

fn address_from_ton(addr: &TonAddress) -> Result<TvmAddress, BocError> {
    TvmAddress::try_from(addr).map_err(|e: TvmAddressFormatError| BocError::Invalid(e.to_string()))
}

/// Returns the TEP-74 default forward payload cell.
pub fn default_forward_payload() -> Result<Cell, BocError> {
    make_zero_bit_cell()
}
