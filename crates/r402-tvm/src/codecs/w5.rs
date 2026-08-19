//! W5R1 wallet construction, signed-body codec, and settlement BoC parse.

use num_bigint::BigUint;
use tonlib_core::TonAddress;
use tonlib_core::cell::{Cell, CellBuilder, EMPTY_CELL};
use tonlib_core::message::{CommonMsgInfo, InternalMessage, TonMessage, TransferMessage};
use tonlib_core::tlb_types::block::out_action::{OutAction, OutList};
use tonlib_core::tlb_types::tlb::TLB;
use tonlib_core::types::TonHash;

use super::common::{BocError, cell_hash_hex, code_from_hex, decode_base64_boc};
use super::jetton::{ParsedJettonTransfer, parse_jetton_transfer};
use crate::chain::{TvmAddress, TvmChainReference};
use crate::exact::error::{
    ERR_EXACT_TVM_INVALID_JETTON_TRANSFER, ERR_EXACT_TVM_INVALID_SETTLEMENT_BOC,
    ERR_EXACT_TVM_INVALID_W5_ACTIONS, ERR_EXACT_TVM_INVALID_W5_MESSAGE,
};
use crate::{
    INTERNAL_SIGNED_OP, SEND_MODE_IGNORE_ERRORS, SEND_MODE_PAY_FEES_SEPARATELY, SEND_MSG_OP,
    W5R1_CODE_HASH, W5R1_CODE_HEX,
};

/// W5R1 data cell fields.
#[derive(Debug, Clone, Copy)]
pub struct W5InitData {
    /// `signature_allowed` flag.
    pub signature_allowed: bool,
    /// Wallet seqno.
    pub seqno: u32,
    /// Network-bound wallet id.
    pub wallet_id: u32,
    /// Ed25519 public key.
    pub public_key: [u8; 32],
    /// Extensions dict present.
    pub has_extensions: bool,
}

/// Parsed client settlement BoC.
#[derive(Debug, Clone)]
pub struct ParsedTvmSettlement {
    /// Client W5 wallet (internal-message destination).
    pub payer: TvmAddress,
    /// Signed `walletId`.
    pub wallet_id: u32,
    /// Unix `validUntil`.
    pub valid_until: u32,
    /// Signed seqno.
    pub seqno: u32,
    /// Hash of the settlement root cell (cache key).
    pub settlement_hash: String,
    /// Signed W5 body cell (relayed as-is).
    pub body: Cell,
    /// Hash of the unsigned W5 slice (signature preimage).
    pub signed_slice_hash: TonHash,
    /// Ed25519 signature (64 bytes).
    pub signature: [u8; 64],
    /// Optional `stateInit` (undeployed wallets).
    pub state_init: Option<StateInitCells>,
    /// Parsed jetton transfer.
    pub transfer: ParsedJettonTransfer,
}

/// Code + data cells of a TON `StateInit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateInitCells {
    /// Code cell.
    pub code: Cell,
    /// Data cell.
    pub data: Cell,
}

impl StateInitCells {
    /// Hex hash of the code cell.
    #[must_use]
    pub fn code_hash_hex(&self) -> String {
        cell_hash_hex(&self.code)
    }
}

/// Derives the W5R1 `walletId` for `network` / `workchain` / `subwallet_number`.
///
/// # Errors
///
/// Returns [`BocError`] if the context cell cannot be built.
pub fn make_w5r1_wallet_id(
    chain: TvmChainReference,
    workchain: i8,
    subwallet_number: u16,
) -> Result<u32, BocError> {
    let context = CellBuilder::new()
        .store_bit(true)?
        .store_i8(8, workchain)?
        .store_u8(8, 0)?
        .store_u16(15, subwallet_number)?
        .build()?;
    let mut parser = context.parser();
    let context_i32 = parser.load_i32(32)?;
    Ok((chain.global_id() as u32) ^ (context_i32 as u32))
}

/// Builds W5R1 `StateInit` data for `public_key` / `wallet_id`.
///
/// # Errors
///
/// Returns [`BocError`] if cells cannot be built.
pub fn build_w5r1_state_init(
    public_key: &[u8; 32],
    wallet_id: u32,
) -> Result<StateInitCells, BocError> {
    let code = code_from_hex(W5R1_CODE_HEX)?;
    if cell_hash_hex(&code) != W5R1_CODE_HASH {
        return Err(BocError::Invalid(
            "unexpected W5R1 wallet code hash".to_owned(),
        ));
    }
    let data = CellBuilder::new()
        .store_bit(true)?
        .store_u32(32, 0)?
        .store_u32(32, wallet_id)?
        .store_slice(public_key)?
        .store_bit(false)?
        .build()?;
    Ok(StateInitCells { code, data })
}

/// Contract address for `state_init` on `workchain`.
///
/// # Errors
///
/// Returns [`BocError`] if the address cannot be derived.
pub fn address_from_state_init(
    state_init: &StateInitCells,
    workchain: i32,
) -> Result<TvmAddress, BocError> {
    let addr = TonAddress::derive(
        workchain,
        state_init.code.clone().to_arc(),
        state_init.data.clone().to_arc(),
    )?;
    TvmAddress::try_from(&addr).map_err(|e| BocError::Invalid(e.to_string()))
}

/// Parses W5 data from a `StateInit` data cell.
///
/// # Errors
///
/// Returns [`BocError`] if the data cell is not W5R1 init data.
pub fn parse_w5_init_data(state_init: &StateInitCells) -> Result<W5InitData, BocError> {
    let mut data = state_init.data.parser();
    let result = W5InitData {
        signature_allowed: data.load_bit()?,
        seqno: data.load_u32(32)?,
        wallet_id: data.load_u32(32)?,
        public_key: {
            let bytes = data.load_bytes(32)?;
            let mut key = [0u8; 32];
            if bytes.len() != 32 {
                return Err(BocError::Invalid(
                    ERR_EXACT_TVM_INVALID_W5_MESSAGE.to_owned(),
                ));
            }
            key.copy_from_slice(&bytes);
            key
        },
        has_extensions: data.load_bit()?,
    };
    if data.remaining_bits() != 0 || data.remaining_refs() != 0 {
        return Err(BocError::Invalid(
            ERR_EXACT_TVM_INVALID_W5_MESSAGE.to_owned(),
        ));
    }
    Ok(result)
}

/// Serializes a W5 `send_msg` action cell.
///
/// # Errors
///
/// Returns [`BocError`] if the cell cannot be built.
pub fn serialize_send_msg_action(message: &Cell, mode: u8) -> Result<Cell, BocError> {
    Ok(CellBuilder::new()
        .store_u32(32, SEND_MSG_OP)?
        .store_u8(8, mode)?
        .store_reference(&message.clone().to_arc())?
        .build()?)
}

/// Serializes an out-list so the last action is outermost (W5 / Highload order).
///
/// # Errors
///
/// Returns [`BocError`] if the cell cannot be built.
pub fn serialize_out_list(actions: &[Cell]) -> Result<Cell, BocError> {
    let mut out_list = EMPTY_CELL.clone();
    for action in actions {
        out_list = CellBuilder::new()
            .store_reference(&out_list.clone().to_arc())?
            .store_cell(action)?
            .build()?;
    }
    Ok(out_list)
}

/// Builds an unsigned W5 body and appends `signature`.
///
/// # Errors
///
/// Returns [`BocError`] if the body cannot be built.
pub fn pack_w5_signed_body(
    opcode: u32,
    wallet_id: u32,
    valid_until: u32,
    seqno: u32,
    actions: &Cell,
    signature: &[u8; 64],
) -> Result<Cell, BocError> {
    let unsigned = unsigned_w5_body(opcode, wallet_id, valid_until, seqno, Some(actions))?;
    Ok(CellBuilder::new()
        .store_cell(&unsigned)?
        .store_slice(signature)?
        .build()?)
}

/// Hash of the unsigned W5 body (Ed25519 preimage).
///
/// # Errors
///
/// Returns [`BocError`] if the unsigned body cannot be built.
pub fn unsigned_w5_body(
    opcode: u32,
    wallet_id: u32,
    valid_until: u32,
    seqno: u32,
    actions: Option<&Cell>,
) -> Result<Cell, BocError> {
    let mut builder = CellBuilder::new();
    builder.store_u32(32, opcode)?;
    builder.store_u32(32, wallet_id)?;
    builder.store_u32(32, valid_until)?;
    builder.store_u32(32, seqno)?;
    if let Some(actions) = actions {
        builder.store_bit(true)?;
        builder.store_reference(&actions.clone().to_arc())?;
    } else {
        builder.store_bit(false)?;
    }
    builder.store_bit(false)?;
    Ok(builder.build()?)
}

/// Builds a bounceable internal message.
///
/// # Errors
///
/// Returns [`BocError`] if the message cannot be built.
pub fn build_internal_message(
    dest: &TonAddress,
    value: u128,
    bounce: bool,
    state_init: Option<&StateInitCells>,
    body: &Cell,
) -> Result<Cell, BocError> {
    let info = CommonMsgInfo::InternalMessage(InternalMessage {
        ihr_disabled: true,
        bounce,
        bounced: false,
        src: TonAddress::NULL,
        dest: dest.clone(),
        value: BigUint::from(value),
        ihr_fee: BigUint::from(0u32),
        fwd_fee: BigUint::from(0u32),
        created_lt: 0,
        created_at: 0,
    });
    let mut msg = TransferMessage::new(info, body.clone().to_arc());
    if let Some(init) = state_init {
        let init_cell = state_init_cell(init)?;
        msg.with_state_init(init_cell);
    }
    Ok(msg.build().map_err(|e| BocError::Invalid(e.to_string()))?)
}

/// Builds an external-in message.
///
/// # Errors
///
/// Returns [`BocError`] if the message cannot be built.
pub fn build_external_message(
    dest: &TonAddress,
    state_init: Option<&StateInitCells>,
    body: &Cell,
) -> Result<Cell, BocError> {
    use tonlib_core::message::ExternalIncomingMessage;

    let info = CommonMsgInfo::ExternalIncomingMessage(ExternalIncomingMessage {
        src: TonAddress::NULL,
        dest: dest.clone(),
        import_fee: BigUint::from(0u32),
    });
    let mut msg = TransferMessage::new(info, body.clone().to_arc());
    if let Some(init) = state_init {
        msg.with_state_init(state_init_cell(init)?);
    }
    Ok(msg.build().map_err(|e| BocError::Invalid(e.to_string()))?)
}

fn state_init_cell(init: &StateInitCells) -> Result<Cell, BocError> {
    use tonlib_core::tlb_types::block::state_init::StateInit;

    let state = StateInit::new(init.code.clone().to_arc(), init.data.clone().to_arc());
    Ok(state.to_cell()?)
}

/// Parses a client settlement BoC into a W5 `internal_signed` jetton transfer.
///
/// # Errors
///
/// Returns [`BocError`] with a wire `invalidReason` string on malformed input.
pub fn parse_exact_tvm_payload(settlement_boc: &str) -> Result<ParsedTvmSettlement, BocError> {
    let root = decode_base64_boc(settlement_boc)
        .map_err(|_| BocError::Invalid(ERR_EXACT_TVM_INVALID_SETTLEMENT_BOC.to_owned()))?;
    let message = TransferMessage::parse(&root)
        .map_err(|_| BocError::Invalid(ERR_EXACT_TVM_INVALID_SETTLEMENT_BOC.to_owned()))?;
    let CommonMsgInfo::InternalMessage(info) = &message.common_msg_info else {
        return Err(BocError::Invalid(
            ERR_EXACT_TVM_INVALID_SETTLEMENT_BOC.to_owned(),
        ));
    };
    let payer = TvmAddress::try_from(&info.dest).map_err(|e| BocError::Invalid(e.to_string()))?;
    let body = (*message.body).clone();
    let mut body_slice = body.parser();
    if body_slice.remaining_bits() < 32 {
        return Err(BocError::Invalid(
            ERR_EXACT_TVM_INVALID_W5_MESSAGE.to_owned(),
        ));
    }
    let opcode = body_slice.load_u32(32)?;
    if opcode != INTERNAL_SIGNED_OP {
        return Err(BocError::Invalid(
            ERR_EXACT_TVM_INVALID_W5_MESSAGE.to_owned(),
        ));
    }
    let wallet_id = body_slice.load_u32(32)?;
    let valid_until = body_slice.load_u32(32)?;
    let seqno = body_slice.load_u32(32)?;
    let has_actions = body_slice.load_bit()?;
    let actions_cell = if has_actions {
        Some((*body_slice.next_reference()?).clone())
    } else {
        None
    };
    let has_extra = body_slice.load_bit()?;
    if has_extra {
        return Err(BocError::Invalid(
            ERR_EXACT_TVM_INVALID_W5_ACTIONS.to_owned(),
        ));
    }

    let actions_cell_ref = actions_cell
        .as_ref()
        .ok_or_else(|| BocError::Invalid(ERR_EXACT_TVM_INVALID_W5_ACTIONS.to_owned()))?;
    let out_list = OutList::from_cell(actions_cell_ref)
        .map_err(|_| BocError::Invalid(ERR_EXACT_TVM_INVALID_W5_ACTIONS.to_owned()))?;
    let OutList::Some(action_node) = out_list else {
        return Err(BocError::Invalid(
            ERR_EXACT_TVM_INVALID_W5_ACTIONS.to_owned(),
        ));
    };
    let inner = OutList::from_cell(&action_node.prev.0)
        .map_err(|_| BocError::Invalid(ERR_EXACT_TVM_INVALID_W5_ACTIONS.to_owned()))?;
    if inner != OutList::Empty {
        return Err(BocError::Invalid(
            ERR_EXACT_TVM_INVALID_W5_ACTIONS.to_owned(),
        ));
    }
    let OutAction::SendMsg(send) = action_node.action else {
        return Err(BocError::Invalid(
            ERR_EXACT_TVM_INVALID_W5_ACTIONS.to_owned(),
        ));
    };
    let allowed = [
        SEND_MODE_PAY_FEES_SEPARATELY,
        SEND_MODE_PAY_FEES_SEPARATELY + SEND_MODE_IGNORE_ERRORS,
    ];
    if !allowed.contains(&send.mode) {
        return Err(BocError::Invalid(
            ERR_EXACT_TVM_INVALID_W5_ACTIONS.to_owned(),
        ));
    }
    let out_msg = TransferMessage::parse(&send.out_msg)
        .map_err(|_| BocError::Invalid(ERR_EXACT_TVM_INVALID_W5_ACTIONS.to_owned()))?;
    let CommonMsgInfo::InternalMessage(out_info) = &out_msg.common_msg_info else {
        return Err(BocError::Invalid(
            ERR_EXACT_TVM_INVALID_W5_ACTIONS.to_owned(),
        ));
    };
    if !out_info.bounce {
        return Err(BocError::Invalid(
            ERR_EXACT_TVM_INVALID_W5_ACTIONS.to_owned(),
        ));
    }
    let source_wallet = TvmAddress::try_from(&out_info.dest)
        .map_err(|_| BocError::Invalid(ERR_EXACT_TVM_INVALID_W5_ACTIONS.to_owned()))?;
    let mut transfer = parse_jetton_transfer(&source_wallet, &out_msg.body)
        .map_err(|_| BocError::Invalid(ERR_EXACT_TVM_INVALID_JETTON_TRANSFER.to_owned()))?;
    transfer.attached_ton_amount = super::jetton::coins_to_u128(&out_info.value)?;

    let sig_bytes = body_slice.load_bytes(64)?;
    if sig_bytes.len() != 64 || body_slice.remaining_bits() != 0 || body_slice.remaining_refs() != 0
    {
        return Err(BocError::Invalid(
            ERR_EXACT_TVM_INVALID_W5_MESSAGE.to_owned(),
        ));
    }
    let mut signature = [0u8; 64];
    signature.copy_from_slice(&sig_bytes);

    let unsigned = unsigned_w5_body(opcode, wallet_id, valid_until, seqno, actions_cell.as_ref())?;

    let state_init = match message.state_init {
        Some(cell) => Some(parse_state_init_cells(&cell)?),
        None => None,
    };

    Ok(ParsedTvmSettlement {
        payer,
        wallet_id,
        valid_until,
        seqno,
        settlement_hash: cell_hash_hex(&root),
        body,
        signed_slice_hash: unsigned.cell_hash(),
        signature,
        state_init,
        transfer,
    })
}

fn parse_state_init_cells(cell: &tonlib_core::cell::ArcCell) -> Result<StateInitCells, BocError> {
    use tonlib_core::tlb_types::block::state_init::StateInit;

    let init = StateInit::from_cell(cell)
        .map_err(|_| BocError::Invalid(ERR_EXACT_TVM_INVALID_W5_MESSAGE.to_owned()))?;
    let code = init
        .code
        .ok_or_else(|| BocError::Invalid(ERR_EXACT_TVM_INVALID_W5_MESSAGE.to_owned()))?
        .0;
    let data = init
        .data
        .ok_or_else(|| BocError::Invalid(ERR_EXACT_TVM_INVALID_W5_MESSAGE.to_owned()))?
        .0;
    Ok(StateInitCells {
        code: (*code).clone(),
        data: (*data).clone(),
    })
}

/// Returns `true` if `code_hash` is the W5R1 allowlist entry.
#[must_use]
pub fn is_allowed_client_code(code_hash: &str) -> bool {
    code_hash == W5R1_CODE_HASH
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn wallet_id_matches_tonlib_defaults() {
        let mainnet = make_w5r1_wallet_id(TvmChainReference::MAINNET, 0, 0).unwrap();
        assert_eq!(mainnet, 0x7FFF_FF11);
        let testnet = make_w5r1_wallet_id(TvmChainReference::TESTNET, 0, 0).unwrap();
        assert_eq!(testnet, 0x7FFF_FFFD);
    }

    #[test]
    fn w5r1_code_hash_matches_constant() {
        let code = code_from_hex(W5R1_CODE_HEX).unwrap();
        assert_eq!(cell_hash_hex(&code), W5R1_CODE_HASH);
    }
}
